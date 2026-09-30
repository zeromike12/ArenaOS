//! `fstest` — the filesystem-service client (M5.3, ADR-0023). Spawn-
//! registry image 5, spawned ONLY by the m5 suite's fs_service test
//! with two kernel-literal grants:
//!
//! - slot 0: `Endpoint` (WRITE — the call side of fsd's FS service),
//! - slot 1: `Endpoint` (WRITE — storaged's call side, for the final
//!   poison that shuts the block service down cleanly).
//!
//! The whole point of this program is the FILESYSTEM BOUNDARY: every
//! step runs in ring 3, in a process that has never seen a disk
//! register. M5.4 gave it a BRANCH PROBE (ADR-0023): it OPENs
//! "arena.txt" before doing anything else, and the answer says what
//! kind of volume this boot inherited —
//!
//! FRESH (OPEN → NOT_FOUND): the full create contract —
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
//!   Device-operation contract: 34 (mount 12 — superblock 1, commit
//!   slots 2, superseded-slot probe 1, objtab 4, bitmap 4 — plus
//!   CREATE commit 9, WRITE 11, READ 2).
//!
//! PERSISTED (OPEN → OK): the file was committed by an EARLIER boot
//! and survived the reboot — the two-boot persistence proof runs
//! inside every dirty boot's suite. No write is issued at all:
//! READ + byte-for-byte verify, a TOLERANT LS walk (arena.txt must
//! appear at exactly 512 bytes; other committed files legitimately
//! share the namespace), CLOSE, the same clean shutdowns, exit 43.
//! Device-operation contract: 14 (mount 12 + OPEN 0 + READ 2).
//!
//! Both branches assert fsd's lifetime disk-operation count AND
//! storaged's completion count against their contract before exiting
//! — the kernel's relay-delivery count is the third independent
//! number that must agree (m5.rs).
//!
//! Exit codes (the m5 contract): 42 verified (fresh), 43 verified
//! (persisted), 69 buffer setup, 70 CREATE, 71 WRITE, 72 CLOSE,
//! 73 re-OPEN, 74 READ, 75 the read-back pattern MISMATCHED, 76 the
//! LS walk, 77 the fsd shutdown, 78 the storaged poison, 97 console
//! refused, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;
use arena_lib::fs;

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
    // This independent FS consumer links the SAME checked library as
    // permissiond. The raw escape hatch preserves the historical exact
    // operation count and the receiver-gated SHUTDOWN fixture.
    match fs::Client::new(slot).request(op, w1, cap, msg) {
        Ok(reply) => (reply.status, reply.value),
        Err(fs::Error::Ipc(arena_lib::ipc::Error::Transport(r))) => {
            log_line(|o| {
                o.str("fstest: call(op ");
                o.u64(op);
                o.str(") transport refused: ");
                o.i64(r);
            });
            fail(fail_code, "the service call was refused");
        }
        Err(fs::Error::Ipc(arena_lib::ipc::Error::ReturnedCap)) =>
            fail(fail_code, "fsd sent a cap back (the protocol never does)"),
        Err(fs::Error::Input) => fail(fail_code, "fs client input refused"),
    }
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

/// The two verified device-operation contracts (m5.rs derives the
/// same numbers from the AFS1 layout; the kernel's relay count is
/// the third independent witness).
const OPS_FRESH: u64 = 34; // mount 12 + CREATE 9 + WRITE 11 + READ 2
const OPS_PERSISTED: u64 = 14; // mount 12 + OPEN 0 + READ 2

/// CLEAR the frame, READ the whole test file into it through the
/// forwarded-cap path, and verify every byte against the shared
/// deterministic pattern (the bytes must travel disk → device →
/// THIS frame; fsd never sees them — a lent cap cannot be mapped).
unsafe fn read_verify(buf: u64, fh: u64, msg: &mut [u8; MSG_BYTES]) {
    // SAFETY: `buf` is this process's own mapped frame; `msg` is this
    // thread's own inline buffer (nested in the entry's unsafe).
    unsafe {
        for i in 0..TEST_LEN {
            *((buf + i as u64) as *mut u8) = 0;
        }
        msg_set_len(msg, TEST_LEN as u64);
        let (st, n) = fs_call(
            SLOT_FS,
            FS_OP_READ,
            fs_rw_w1(fh, 0),
            SLOT_BUF_LENT,
            msg,
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
        for i in 0..TEST_LEN {
            let got = *((buf + i as u64) as *const u8);
            if got != pattern_byte(i) {
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
        }
        log_line(|o| {
            o.str("fstest: READ ");
            o.u64(TEST_LEN as u64);
            o.str(" bytes → all verified byte-for-byte through fsd+storaged");
        });
    }
}

/// Walk LS to the end cursor. `strict_single` is the FRESH-volume
/// contract: exactly one dirent (the test file). A persisted volume
/// legitimately holds other committed files (the shell's, earlier
/// suites'), so the dirty walk tolerates them — but arena.txt must
/// still appear at exactly TEST_LEN bytes, or the walk fails.
unsafe fn ls_walk(msg: &mut [u8; MSG_BYTES], strict_single: bool) {
    // SAFETY: `msg` is this thread's own buffer; the dirent reads are
    // unaligned loads of owned memory (the FS protocol's layout).
    unsafe {
        let mut cursor = 0u64;
        let mut files = 0u32;
        let mut found = false;
        loop {
            let (st, _) = fs_call(SLOT_FS, FS_OP_LS, cursor, CAP_NONE, msg, EXIT_LS);
            if st != FS_OK {
                log_line(|o| {
                    o.str("fstest: LS status ");
                    o.i64(st as i64);
                });
                fail(EXIT_LS, "LS was not served");
            }
            let (next, size, nlen) = (
                core::ptr::read_unaligned(msg.as_ptr() as *const u32),
                core::ptr::read_unaligned(msg.as_ptr().add(4) as *const u64),
                core::ptr::read_unaligned(msg.as_ptr().add(12) as *const u32),
            );
            if next == FS_CURSOR_END {
                break;
            }
            files += 1;
            if nlen as usize == TEST_NAME.len() && msg[16..16 + nlen as usize] == *TEST_NAME {
                if size != TEST_LEN as u64 {
                    log_line(|o| {
                        o.str("fstest: 'arena.txt' dirent size ");
                        o.u64(size);
                        o.str(", expected ");
                        o.u64(TEST_LEN as u64);
                    });
                    fail(EXIT_LS, "the test file's committed size was wrong");
                }
                found = true;
            }
            cursor = u64::from(next);
        }
        if !found {
            fail(EXIT_LS, "the LS walk never showed the test file");
        }
        if strict_single && files != 1 {
            log_line(|o| {
                o.str("fstest: LS walk showed ");
                o.u64(u64::from(files));
                o.str(" file(s); a fresh volume holds exactly one");
            });
            fail(
                EXIT_LS,
                "the fresh-volume LS walk did not end after the test file",
            );
        }
        log_line(|o| {
            o.str("fstest: LS walk: 'arena.txt' ");
            o.u64(TEST_LEN as u64);
            o.str(" bytes present — ");
            o.u64(u64::from(files));
            o.str(" file(s) total");
        });
    }
}

/// SHUTDOWN fsd (it replies with its lifetime disk-operation count),
/// then poison storaged (it replies with its used-ring completions).
/// BOTH counters must equal this branch's contract — and the kernel
/// independently asserts its relay-delivery count against the same
/// number: three witnesses, one contract.
unsafe fn shutdown_services(msg: &mut [u8; MSG_BYTES], expected_ops: u64) {
    // SAFETY: `msg` is this thread's own buffer; `reply` its own
    // stack; the caps are the granted slots.
    unsafe {
        if !diagnostic_refused(SLOT_FS, FS_OP_SHUTDOWN, 0, CAP_NONE, FS_ERR_BAD_OP)
            || !diagnostic_refused(SLOT_FS, FS_OP_SHUTDOWN, 0, 3, FS_ERR_BAD_OP)
            || !diagnostic_refused(SLOT_BLK, u64::MAX, block_req_w1(OP_SHUTDOWN, 0), 2, VIRTIO_BLK_S_UNSUPP)
        { fail(EXIT_FSD_SHUTDOWN, "ordinary or wrong diagnostic marker authorized poison"); }
        log_line(|o| o.str("fstest: fsd and storaged rejected missing/wrong-object poison markers"));
        let (st, disk_ops) = fs_call(SLOT_FS, FS_OP_SHUTDOWN, 0, 2, msg, EXIT_FSD_SHUTDOWN);
        if st != FS_OK || disk_ops != expected_ops {
            log_line(|o| {
                o.str("fstest: SHUTDOWN status ");
                o.i64(st as i64);
                o.str(" disk_ops ");
                o.u64(disk_ops);
                o.str(", contract says ");
                o.u64(expected_ops);
            });
            fail(
                EXIT_FSD_SHUTDOWN,
                "the fsd shutdown was not served per contract",
            );
        }
        let mut reply = [0u64; 3];
        let r = syscall6(
            SYS_IPC_CALL,
            SLOT_BLK,
            u64::MAX,
            block_req_w1(OP_SHUTDOWN, 0),
            3,
            reply.as_mut_ptr() as u64,
            0,
        );
        if r < 0 || reply[0] != VIRTIO_BLK_S_OK || reply[1] != expected_ops {
            log_line(|o| {
                o.str("fstest: poison transport ");
                o.i64(r);
                o.str(" status ");
                o.u64(reply[0]);
                o.str(" completions ");
                o.u64(reply[1]);
                o.str(", contract says ");
                o.u64(expected_ops);
            });
            fail(
                EXIT_BLK_POISON,
                "the storaged poison was refused or off-contract",
            );
        }
        log_line(|o| {
            o.str("fstest: fsd reported ");
            o.u64(disk_ops);
            o.str(" disk operation(s); storaged reported ");
            o.u64(reply[1]);
            o.str(" completion(s) — the contract's ");
            o.u64(expected_ops);
        });
    }
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
        log_line(|o| o.str("fstest: filesystem-service client starting (M5.3/5.4, ADR-0023)"));

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

        // 1. THE BRANCH PROBE (M5.4): OPEN the test file before
        //    touching anything. NOT_FOUND → a fresh volume, run the
        //    create contract. OK → the file was committed by an
        //    EARLIER BOOT and survived: verify it with no write at
        //    all — the two-boot persistence proof, inside the suite.
        msg_set_name(&mut msg, TEST_NAME);
        let (st, fh0) = fs_call(SLOT_FS, FS_OP_OPEN, 0, CAP_NONE, &mut msg, EXIT_OPEN);

        if st == FS_OK {
            log_line(|o| {
                o.str("fstest: OPEN 'arena.txt' succeeded BEFORE any create — this volume PERSISTED from an earlier boot (fh ");
                o.u64(fh0);
                o.str(")");
            });
            read_verify(buf, fh0, &mut msg);
            ls_walk(&mut msg, false);
            let (st, _) = fs_call(SLOT_FS, FS_OP_CLOSE, fh0, CAP_NONE, &mut msg, EXIT_CLOSE);
            if st != FS_OK {
                fail(EXIT_CLOSE, "CLOSE was not served");
            }
            shutdown_services(&mut msg, OPS_PERSISTED);
            log_line(|o| {
                o.str("fstest: PASS (persisted) — the committed file SURVIVED THE REBOOT: opened, read back, verified byte-for-byte, zero writes issued");
            });
            // SAFETY: thread_exit diverges; 43 is the persisted-volume
            // verified-success code (m5.rs asserts the 14-op contract
            // for exactly this exit).
            syscall1(SYS_THREAD_EXIT, EXIT_OK_PERSISTED);
            #[allow(unreachable_code)]
            loop {
                core::hint::spin_loop();
            }
        }
        if st != FS_ERR_NOT_FOUND {
            log_line(|o| {
                o.str("fstest: the branch-probe OPEN answered ");
                o.i64(st as i64);
            });
            fail(
                EXIT_OPEN,
                "the branch probe got a status from nowhere in the contract",
            );
        }
        log_line(|o| {
            o.str("fstest: 'arena.txt' absent — fresh volume: create→write→close→re-open→read→verify→ls");
        });

        // 2. CREATE the test file — the name rides the inline message.
        //    (Set it AGAIN: the reply cycle copies fsd's response msg
        //    back over this buffer, so the probe's name is gone.)
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

        // 3. WRITE the pattern — the device reads THIS frame through
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

        // 4. CLOSE, then re-OPEN by name: the file must come back from
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

        // 5. Read back + verify, the strict fresh-volume LS walk,
        //    then the two clean service shutdowns on the 34-op
        //    contract.
        read_verify(buf, fh2, &mut msg);
        ls_walk(&mut msg, true);
        let (st, _) = fs_call(SLOT_FS, FS_OP_CLOSE, fh2, CAP_NONE, &mut msg, EXIT_CLOSE);
        if st != FS_OK {
            fail(EXIT_CLOSE, "the second CLOSE was not served");
        }
        shutdown_services(&mut msg, OPS_FRESH);
        log_line(|o| {
            o.str("fstest: PASS (fresh) — create/write/read/close ran entirely in ring 3 and the file persisted across re-open");
        });
        // SAFETY: thread_exit diverges; 42 is the fresh-volume
        // verified-success code.
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
        #[allow(unreachable_code)]
        loop {
            core::hint::spin_loop();
        }
    }
}
