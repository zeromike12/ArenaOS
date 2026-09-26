//! `blktest` — the block-service client (M5.2, ADR-0022). Spawn-
//! registry image 3, spawned ONLY by the m5 suite's block_service test
//! with one kernel-literal grant:
//!
//! - slot 0: `Endpoint` (WRITE — the call side of storaged's service).
//!
//! The whole point of this program is the SERVICE BOUNDARY: it is a
//! different process from the driver, it never sees a device register,
//! and its buffer is never copied by the kernel — it allocates one
//! frame (SYS_ALLOC_FRAME), copies its cap (SYS_CAP_COPY: the copy is
//! LENT, the original stays owning), self-maps the ORIGINAL (the map
//! consumes it — ownership moves to this address space, the window
//! stays valid), and hands the COPY to storaged with the call. The
//! device DMAs this process's own page in both directions:
//!
//!   1. fill a 512-byte deterministic pattern into the buffer,
//!   2. WRITE sector 0 through the service (buffer lent with the call),
//!   3. CLEAR the buffer — the read-back must come from the disk, not
//!      from leftovers,
//!   4. READ sector 0 through the service into the same buffer,
//!   5. verify the pattern byte-for-byte,
//!   6. poison the driver (OP_SHUTDOWN) so it exits cleanly, exit 42.
//!
//! Exit codes (the m5 contract): 42 verified, 43 alloc/copy/map
//! refused, 44 the WRITE call refused, 45 WRITE error status, 46 the
//! READ call refused, 47 READ error status, 48 the read-back pattern
//! MISMATCHED, 49 the poison call refused, 97 console refused,
//! 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "abi.rs"]
mod abi;
use abi::*;

/// The grant layout (m5.rs block_service): slot 0 = endpoint call side.
const SLOT_EP: u64 = 0;
/// Slot for the owned buffer cap (consumed by the self-map).
const SLOT_BUF: u64 = 1;
/// Slot for the LENT copy that travels with the calls.
const SLOT_BUF_LENT: u64 = 2;

const EXIT_SETUP: u64 = 43;
const EXIT_WRITE_CALL: u64 = 44;
const EXIT_WRITE_STATUS: u64 = 45;
const EXIT_READ_CALL: u64 = 46;
const EXIT_READ_STATUS: u64 = 47;
const EXIT_MISMATCH: u64 = 48;
const EXIT_POISON: u64 = 49;

/// The sector this test drives (the scratch disk is zero-filled; any
/// in-range sector proves the path — 0 is where fsd will put its
/// superblock, so we claim it early).
const TEST_SECTOR: u64 = 0;

/// The deterministic pattern byte for offset `i` (no allocator, no
/// randomness — the same function fills and verifies).
fn pattern_byte(i: usize) -> u8 {
    (i as u8).wrapping_mul(31).wrapping_add(0x5A) ^ ((i >> 3) as u8)
}

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("blktest: FAIL: ");
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
    write_str("blktest: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

/// One synchronous block request through the service boundary: call
/// with the LENT buffer cap attached, check the transport, return the
/// device's status word.
fn request(op: u64, sector: u64, send_slot: u64, call_fail: u64, status_fail: u64) -> u64 {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply` is on this thread's own
    // (registered) stack; the endpoint cap is the granted slot 0.
    let r = unsafe {
        syscall5(
            SYS_IPC_CALL,
            SLOT_EP,
            sector,
            op,
            send_slot,
            reply.as_mut_ptr() as u64,
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("blktest: call(op ");
            o.u64(op);
            o.str(") returned ");
            o.i64(r);
        });
        fail(call_fail, "the service call was refused");
    }
    if reply[0] != VIRTIO_BLK_S_OK {
        log_line(|o| {
            o.str("blktest: device status ");
            o.u64(reply[0]);
            o.str(" for op ");
            o.u64(op);
        });
        fail(status_fail, "the device reported an error status");
    }
    reply[1]
}

/// The client entry: the spawn protocol's first thread lands here at
/// ring 3 with slot 0 holding the endpoint's call side.
///
/// # Safety
/// As the driver's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, one grant in slot 0. The body is ABI v1 wrappers over
/// this image's own statics, stack, and allocated window.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics, stack, and granted/allocated windows.
    // Single-threaded, no aliases.
    unsafe {
        log_line(|o| {
            o.str(
                "blktest: block-service client starting (write→read-back→verify through storaged)",
            )
        });

        // 1. The buffer: one owned frame; the cap is COPIED before the
        //    map consumes the original, so the copy can travel with the
        //    calls while this process keeps its window (ADR-0022's
        //    lend-keep pattern).
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
            o.str("blktest: buffer frame ");
            o.hex(phys as u64);
            o.str(" mapped at ");
            o.hex(buf);
            o.str(" (lent copy in slot ");
            o.u64(SLOT_BUF_LENT);
            o.str(")");
        });

        // 2. Fill the pattern.
        for i in 0..SECTOR_BYTES {
            *((buf + i as u64) as *mut u8) = pattern_byte(i);
        }

        // 3. WRITE through the service — the device reads THIS page.
        let wlen = request(
            OP_WRITE,
            TEST_SECTOR,
            SLOT_BUF_LENT,
            EXIT_WRITE_CALL,
            EXIT_WRITE_STATUS,
        );
        log_line(|o| {
            o.str("blktest: WRITE sector ");
            o.u64(TEST_SECTOR);
            o.str(" → status OK (device wrote ");
            o.u64(wlen);
            o.str(" bytes to the used ring)");
        });

        // 4. CLEAR the buffer: the read-back must be the DISK's answer,
        //    not this process's leftovers.
        for i in 0..SECTOR_BYTES {
            *((buf + i as u64) as *mut u8) = 0;
        }

        // 5. READ through the service — the device writes THIS page.
        let rlen = request(
            OP_READ,
            TEST_SECTOR,
            SLOT_BUF_LENT,
            EXIT_READ_CALL,
            EXIT_READ_STATUS,
        );

        // 6. Verify byte-for-byte.
        let mut mismatch: Option<(usize, u8)> = None;
        for i in 0..SECTOR_BYTES {
            let got = *((buf + i as u64) as *const u8);
            if got != pattern_byte(i) {
                mismatch = Some((i, got));
                break;
            }
        }
        if let Some((i, got)) = mismatch {
            log_line(|o| {
                o.str("blktest: byte ");
                o.u64(i as u64);
                o.str(" read back ");
                o.hex(u64::from(got));
                o.str(", expected ");
                o.hex(u64::from(pattern_byte(i)));
            });
            fail(EXIT_MISMATCH, "the read-back pattern mismatched");
        }
        log_line(|o| {
            o.str("blktest: READ sector ");
            o.u64(TEST_SECTOR);
            o.str(" → status OK (device wrote ");
            o.u64(rlen);
            o.str(" bytes) — all ");
            o.u64(SECTOR_BYTES as u64);
            o.str(" bytes verified across the service boundary");
        });

        // 7. Poison: the driver replies, then exits cleanly by its own
        //    hand (a service is never destroyed with parked threads).
        request(OP_SHUTDOWN, u64::MAX, CAP_NONE, EXIT_POISON, EXIT_POISON);
        log_line(|o| {
            o.str("blktest: PASS — the cycle completed and the service shut down cleanly")
        });
        // SAFETY: thread_exit diverges; 42 is the verified-success code.
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
        #[allow(unreachable_code)]
        loop {
            core::hint::spin_loop();
        }
    }
}
