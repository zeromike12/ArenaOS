//! `fstest` — the filesystem-service client (M5.3, ADR-0023). Spawn-
//! registry image 5, spawned ONLY by the m5 suite's fs_service test
//! with two kernel-literal grants:
//!
//! - slot 0: `Endpoint` (WRITE — the call side of fsd's FS service),
//! - slot 1: `Endpoint` (WRITE — storaged's call side, for the final
//!   poison that shuts the block service down cleanly).
//!
//! The whole point of this program is the FILESYSTEM BOUNDARY: every
//! step below runs in ring 3, in a process that has never seen a disk
//! register —
//!
//!   1. CREATE "arena.txt" (name travels in the IPC v1.1 inline
//!      message; the commit lands on the scratch disk),
//!   2. WRITE 512 pattern bytes: the buffer frame is LENT to fsd,
//!      which FORWARDS the cap to storaged — the device DMAs this
//!      process's own page (zero-copy end to end),
//!   3. CLOSE, then re-OPEN by name — the file must come back from
//!      the mounted on-disk state,
//!   4. CLEAR the frame, READ into it, verify the pattern byte-for-
//!      byte (the bytes traveled disk → device → THIS frame),
//!   5. LS walk: exactly one file, the right name and size,
//!   6. SHUTDOWN fsd, poison storaged — both services exit by their
//!      own hand (a parked server is never destroyed), exit 42.
//!
//! Exit codes (the m5 contract): 42 verified, 69 buffer setup,
//! 70 CREATE, 71 WRITE, 72 CLOSE, 73 re-OPEN, 74 READ, 75 the
//! read-back pattern MISMATCHED, 76 the LS walk, 77 the fsd shutdown,
//! 78 the storaged poison, 97 console refused, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

/// The grant layout (m5.rs fs_service): slot 0 = fsd call side,
/// slot 1 = storaged call side (the final poison only).
const SLOT_FS: u64 = 0;
const SLOT_BLK: u64 = 1;
/// The buffer frame: owned cap (consumed by the self-map) + the LENT
/// copy that travels with the calls (the lend-keep pattern).
const SLOT_BUF: u64 = 8;
const SLOT_BUF_LENT: u64 = 9;

const EXIT_SETUP: u64 = 69;
const EXIT_CREATE: u64 = 70;
const EXIT_WRITE: u64 = 71;
const EXIT_CLOSE: u64 = 72;
const EXIT_OPEN: u64 = 73;
const EXIT_READ: u64 = 74;
const EXIT_MISMATCH: u64 = 75;
const EXIT_LS: u64 = 76;
const EXIT_FSD_SHUTDOWN: u64 = 77;
const EXIT_BLK_POISON: u64 = 78;

/// The file this test creates — `tools/test_m5.py` looks for exactly
/// this name (and the shared pattern) in the committed on-disk state.
const TEST_NAME: &[u8] = b"arena.txt";
const TEST_LEN: usize = SECTOR_BYTES; // one sector's worth of pattern

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("fstest: FAIL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m5 contract.
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write_str("fstest: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

/// One synchronous FS request. `msg` is the IPC v1.1 inline buffer —
/// IN (names, lengths) and OUT (LS dirents); the reply must never
/// carry a cap (data flows through the forwarded LENT frame instead).
fn fs_call(
    slot: u64,
    op: u64,
    w1: u64,
    cap: u64,
    msg: &mut [u8; MSG_BYTES],
    fail_code: u64,
) -> (u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply`/`msg` are on this thread's own
    // (registered) stack; the endpoint cap is a granted slot.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            slot,
            op,
            w1,
            cap,
            reply.as_mut_ptr() as u64,
            msg.as_mut_ptr() as u64,
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("fstest: call(op ");
            o.u64(op);
            o.str(") transport refused: ");
            o.i64(r);
        });
        fail(fail_code, "the service call was refused");
    }
    if reply[2] != CAP_NONE {
        fail(fail_code, "fsd sent a cap back (the protocol never does)");
    }
    (reply[0], reply[1])
}

/// Put `name` into the inline message, NUL-padded.
fn msg_set_name(msg: &mut [u8; MSG_BYTES], name: &[u8]) {
    for b in msg.iter_mut() {
        *b = 0;
    }
    msg[..name.len()].copy_from_slice(name);
}

/// Put a transfer length into the inline message (READ/WRITE input).
fn msg_set_len(msg: &mut [u8; MSG_BYTES], len: u64) {
    for b in msg.iter_mut() {
        *b = 0;
    }
    // SAFETY: msg is this thread's own 64-byte buffer; unaligned-safe
    // write of the little-endian length word.
    unsafe { core::ptr::write_unaligned(msg.as_mut_ptr() as *mut u64, len) };
}

/// The client entry: the spawn protocol's first thread lands here at
/// ring 3 with the two endpoint grants in slots 0/1.
///
/// # Safety
/// As every image's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, the documented grants. The body is ABI v1 wrappers
/// over this image's own statics, stack, and allocated window.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: single-threaded program over this image's own memory and
    // the granted caps; every helper documents its own contract.
    unsafe {
        log_line(|o| {
            o.str("fstest: filesystem-service client starting (create→write→close→re-open→read→verify→ls)")
        });

        // 0. The buffer frame: owned, copied (the LENT copy travels
        //    with the calls), self-mapped (the map CONSUMES the
        //    original — ADR-0022's lend-keep pattern).
        let phys = syscall1(SYS_ALLOC_FRAME, SLOT_BUF);
        if phys <= 0 {
            fail(EXIT_SETUP, "buffer frame allocation refused");
        }
        if syscall3(SYS_CAP_COPY, SLOT_BUF, SLOT_BUF_LENT, RIGHTS_ALL) < 0 {
            fail(EXIT_SETUP, "the buffer cap copy refused");
        }
        let win = syscall2(SYS_MAP_MEMORY, SLOT_BUF, 1);
        if win <= 0 {
            fail(EXIT_SETUP, "the buffer self-map refused");
        }
        let buf = win as u64;
        log_line(|o| {
            o.str("fstest: buffer frame ");
            o.hex(phys as u64);
            o.str(" mapped at ");
            o.hex(buf);
            o.str(" (lent copy in slot ");
            o.u64(SLOT_BUF_LENT);
            o.str(")");
        });

        let mut msg = [0u8; MSG_BYTES];

        // 1. CREATE the test file — the name rides the inline message.
        msg_set_name(&mut msg, TEST_NAME);
        let (st, fh1) = fs_call(SLOT_FS, FS_OP_CREATE, 0, CAP_NONE, &mut msg, EXIT_CREATE);
        if st != FS_OK {
            log_line(|o| {
                o.str("fstest: CREATE status ");
                o.i64(st as i64);
            });
            fail(EXIT_CREATE, "CREATE was not served");
        }
        log_line(|o| {
            o.str("fstest: CREATE arena.txt → fh ");
            o.u64(fh1);
            o.str(" (object committed to the disk)");
        });

        // 2. WRITE the pattern — the device reads THIS frame through
        //    the cap fsd forwards to storaged.
        for i in 0..TEST_LEN {
            *((buf + i as u64) as *mut u8) = pattern_byte(i);
        }
        msg_set_len(&mut msg, TEST_LEN as u64);
        let (st, n) = fs_call(
            SLOT_FS,
            FS_OP_WRITE,
            fs_rw_w1(fh1, 0),
            SLOT_BUF_LENT,
            &mut msg,
            EXIT_WRITE,
        );
        if st != FS_OK || n != TEST_LEN as u64 {
            log_line(|o| {
                o.str("fstest: WRITE status ");
                o.i64(st as i64);
                o.str(" wrote ");
                o.u64(n);
            });
            fail(EXIT_WRITE, "WRITE was not served in full");
        }
        log_line(|o| {
            o.str("fstest: WRITE ");
            o.u64(TEST_LEN as u64);
            o.str(" bytes at offset 0 → committed (the device DMA'd THIS frame)");
        });

        // 3. CLOSE, then re-OPEN by name: the file must come back from
        //    the mounted on-disk state, not from any client leftover.
        let (st, _) = fs_call(SLOT_FS, FS_OP_CLOSE, fh1, CAP_NONE, &mut msg, EXIT_CLOSE);
        if st != FS_OK {
            fail(EXIT_CLOSE, "CLOSE was not served");
        }
        msg_set_name(&mut msg, TEST_NAME);
        let (st, fh2) = fs_call(SLOT_FS, FS_OP_OPEN, 0, CAP_NONE, &mut msg, EXIT_OPEN);
        if st != FS_OK {
            log_line(|o| {
                o.str("fstest: re-OPEN status ");
                o.i64(st as i64);
            });
            fail(EXIT_OPEN, "the file did not survive CLOSE→OPEN");
        }
        log_line(|o| {
            o.str("fstest: re-OPEN arena.txt after CLOSE → fh ");
            o.u64(fh2);
            o.str(" (the file persisted across re-open)");
        });

        // 4. CLEAR the frame, READ into it, verify byte-for-byte. The
        //    bytes must travel disk → device → THIS frame (fsd never
        //    sees them: a lent cap cannot be mapped).
        for i in 0..TEST_LEN {
            *((buf + i as u64) as *mut u8) = 0;
        }
        msg_set_len(&mut msg, TEST_LEN as u64);
        let (st, n) = fs_call(
            SLOT_FS,
            FS_OP_READ,
            fs_rw_w1(fh2, 0),
            SLOT_BUF_LENT,
            &mut msg,
            EXIT_READ,
        );
        if st != FS_OK || n != TEST_LEN as u64 {
            log_line(|o| {
                o.str("fstest: READ status ");
                o.i64(st as i64);
                o.str(" read ");
                o.u64(n);
            });
            fail(EXIT_READ, "READ was not served in full");
        }
        let mut mismatch: Option<(usize, u8)> = None;
        for i in 0..TEST_LEN {
            let got = *((buf + i as u64) as *const u8);
            if got != pattern_byte(i) {
                mismatch = Some((i, got));
                break;
            }
        }
        if let Some((i, got)) = mismatch {
            log_line(|o| {
                o.str("fstest: byte ");
                o.u64(i as u64);
                o.str(" read back ");
                o.hex(u64::from(got));
                o.str(", expected ");
                o.hex(u64::from(pattern_byte(i)));
            });
            fail(EXIT_MISMATCH, "the file contents mismatched");
        }
        log_line(|o| {
            o.str("fstest: READ ");
            o.u64(TEST_LEN as u64);
            o.str(" bytes → all verified byte-for-byte through fsd+storaged");
        });

        // 5. LS walk: exactly one file, right name and size, then the
        //    cursor must report the end.
        let (st, _) = fs_call(SLOT_FS, FS_OP_LS, 0, CAP_NONE, &mut msg, EXIT_LS);
        if st != FS_OK {
            fail(EXIT_LS, "LS was not served");
        }
        // The dirent layout is the FS protocol's (userspace/abi.rs);
        // msg is this thread's own buffer (nested in the entry's
        // unsafe: unaligned reads of owned memory).
        let (next, size, nlen) = (
            core::ptr::read_unaligned(msg.as_ptr() as *const u32),
            core::ptr::read_unaligned(msg.as_ptr().add(4) as *const u64),
            core::ptr::read_unaligned(msg.as_ptr().add(12) as *const u32),
        );
        if nlen as usize != TEST_NAME.len()
            || &msg[16..16 + nlen as usize] != TEST_NAME
            || size != TEST_LEN as u64
        {
            log_line(|o| {
                o.str("fstest: first dirent: nlen ");
                o.u64(u64::from(nlen));
                o.str(" size ");
                o.u64(size);
            });
            fail(EXIT_LS, "the first dirent was not the test file");
        }
        let (st, _) = fs_call(
            SLOT_FS,
            FS_OP_LS,
            u64::from(next),
            CAP_NONE,
            &mut msg,
            EXIT_LS,
        );
        let next2 = core::ptr::read_unaligned(msg.as_ptr() as *const u32);
        if st != FS_OK || next2 != FS_CURSOR_END {
            log_line(|o| {
                o.str("fstest: second LS: status ");
                o.i64(st as i64);
                o.str(" cursor ");
                o.hex(u64::from(next2));
            });
            fail(EXIT_LS, "the LS walk did not end after the test file");
        }
        log_line(|o| {
            o.str("fstest: LS walk: 'arena.txt' ");
            o.u64(size);
            o.str(" bytes — cursor ended after 1 file(s)");
        });

        let (st, _) = fs_call(SLOT_FS, FS_OP_CLOSE, fh2, CAP_NONE, &mut msg, EXIT_CLOSE);
        if st != FS_OK {
            fail(EXIT_CLOSE, "the second CLOSE was not served");
        }

        // 6. SHUTDOWN fsd: it replies with its lifetime disk-operation
        //    count, then exits by its own hand.
        let (st, disk_ops) = fs_call(
            SLOT_FS,
            FS_OP_SHUTDOWN,
            0,
            CAP_NONE,
            &mut msg,
            EXIT_FSD_SHUTDOWN,
        );
        if st != FS_OK || disk_ops == 0 {
            log_line(|o| {
                o.str("fstest: SHUTDOWN status ");
                o.i64(st as i64);
                o.str(" disk_ops ");
                o.u64(disk_ops);
            });
            fail(EXIT_FSD_SHUTDOWN, "the fsd shutdown was not served");
        }

        // 7. Poison storaged (the block service must also exit cleanly
        //    — a parked driver is never destroyed).
        let mut reply = [0u64; 3];
        let r = syscall6(
            SYS_IPC_CALL,
            SLOT_BLK,
            u64::MAX,
            block_req_w1(OP_SHUTDOWN, 0),
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            0,
        );
        if r < 0 || reply[0] != VIRTIO_BLK_S_OK {
            log_line(|o| {
                o.str("fstest: poison transport ");
                o.i64(r);
                o.str(" status ");
                o.u64(reply[0]);
            });
            fail(EXIT_BLK_POISON, "the storaged poison was refused");
        }
        log_line(|o| {
            o.str("fstest: fsd reported ");
            o.u64(disk_ops);
            o.str(" disk operation(s); storaged reported ");
            o.u64(reply[1]);
            o.str(" completion(s)");
        });
        log_line(|o| {
            o.str("fstest: PASS — create/write/read/close ran entirely in ring 3 and the file persisted across re-open")
        });
        // SAFETY: thread_exit diverges; 42 is the verified-success code.
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
        #[allow(unreachable_code)]
        loop {
            core::hint::spin_loop();
        }
    }
}
