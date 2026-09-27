//! ArenaOS entropy-service test client — `rngtest` (M6.2, ADR-0025).
//! Spawn-registry image 9, spawned ONLY by the m6 suite's rng_service
//! test with one grant:
//!
//! - slot 0: `Endpoint` (WRITE — the call side of rngd's service).
//!
//! The proof problem: randomness cannot be asserted against expected
//! values (a fixed expectation is fixed entropy — that would be the
//! fake this project forbids). So rngtest asserts what a genuine
//! entropy source ALWAYS satisfies and a broken/stuck/zeroed device
//! never does:
//!
//!   1. the device wrote the full draw (its own used-ring count),
//!   2. the draw is not all-zero (an unwritten page reads zero — this
//!      catches a descriptor the device ignored),
//!   3. the draw is not a single repeated byte (a stuck source),
//!   4. two independent draws into SEPARATE frames DIFFER (a looping
//!      or cached source).
//!
//! Quality beyond that (distribution, unpredictability) belongs to the
//! host backend and is not claimed here. The kernel side of the proof
//! — one MSI-X relay delivery per draw, exact exit badges, frame-exact
//! teardown — is m6.rs's job.
//!
//! Exit codes (the diagnostic contract with the m6 suite): 42 verified
//! success, 52..57 failure stages, 97 console refused, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

/// The grant layout (m6.rs rng_service): slot 0 = endpoint call side.
const SLOT_EP: u64 = 0;
/// Owned draw-frame slots (the self-map consumes each) and the LENT
/// copies that travel with the GET calls.
const SLOT_BUF_A: u64 = 1;
const SLOT_BUF_B: u64 = 2;
/// The LENT copies that travel with the GET calls — one per frame,
/// COPIED BEFORE the self-map consumes the original (ADR-0022's
/// lend-keep pattern: map consumes, so the copy must exist first).
const SLOT_LENT_A: u64 = 3;
const SLOT_LENT_B: u64 = 4;

const EXIT_SETUP: u64 = 52;
const EXIT_GET_CALL: u64 = 53;
const EXIT_GET_STATUS: u64 = 54;
const EXIT_SHORT: u64 = 55;
const EXIT_NOT_RANDOM: u64 = 56;
const EXIT_POISON: u64 = 57;

/// Each draw fills a whole frame (RNG_DRAW_MAX) — the largest single
/// request the protocol accepts, and the strongest variance evidence
/// per round trip.
const DRAW: u64 = RNG_DRAW_MAX;

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("rngtest: FAIL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m6 contract.
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

fn log(s: &str) {
    log_line(|o| o.str(s));
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write_str("rngtest: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

/// One synchronous request through the service boundary. Returns
/// (reply status word, reply payload word); a refused CALL fails with
/// `call_fail`.
fn request(op: u64, w0: u64, send_slot: u64, call_fail: u64) -> (u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract; `reply` is on this thread's own
    // (registered) stack; the endpoint cap is the granted slot 0.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            w0,
            op,
            send_slot,
            reply.as_mut_ptr() as u64,
            0,
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("rngtest: call(op ");
            o.u64(op);
            o.str(") returned ");
            o.i64(r);
        });
        fail(call_fail, "the service call was refused");
    }
    (reply[0], reply[1])
}

/// Allocate one owned frame, COPY its cap into `lent_slot`, then
/// self-map the original RW. The order is load-bearing: the self-map
/// CONSUMES the cap, so the copy that travels with the GET must be
/// taken first — ADR-0022's lend-keep pattern, exactly as nettest
/// does it. Returns the frame's VA in this image.
///
/// # Safety
/// Ring 3, single-threaded; both slots are free in this image's slot
/// table.
unsafe fn draw_frame(slot: u64, lent_slot: u64) -> u64 {
    // SAFETY: wrapper contract.
    let phys = unsafe { syscall1(SYS_ALLOC_FRAME, slot) };
    if phys <= 0 {
        fail(EXIT_SETUP, "draw frame allocation refused");
    }
    // SAFETY: wrapper contract; the copy must precede the map.
    if unsafe { syscall3(SYS_CAP_COPY, slot, lent_slot, RIGHTS_ALL) } < 0 {
        fail(EXIT_SETUP, "the draw cap copy refused");
    }
    // SAFETY: wrapper contract.
    let win = unsafe { syscall2(SYS_MAP_MEMORY, slot, 1) };
    if win <= 0 {
        fail(EXIT_SETUP, "the draw frame self-map refused");
    }
    win as u64
}

/// Ask rngd to fill the frame named by the LENT cap in `lent_slot`
/// with `DRAW` bytes of device entropy, and check the transport-level
/// facts: OK status and the device's own written count.
fn draw(lent_slot: u64, which: &str) {
    let (st, written) = request(RNG_OP_GET, DRAW, lent_slot, EXIT_GET_CALL);
    if st != RNG_S_OK {
        log_line(|o| {
            o.str("rngtest: RNG_GET status ");
            o.i64(st as i64);
        });
        fail(EXIT_GET_STATUS, "the draw reply carried an error status");
    }
    if written != DRAW {
        log_line(|o| {
            o.str("rngtest: draw ");
            o.str(which);
            o.str(" — device wrote ");
            o.u64(written);
            o.str(" of ");
            o.u64(DRAW);
            o.str(" bytes");
        });
        fail(EXIT_SHORT, "the device wrote fewer bytes than requested");
    }
}

/// The variance checks on ONE draw: a frame the device never touched
/// reads all-zero, and a stuck source repeats a single byte. Both are
/// properties every genuine entropy source violates with probability
/// 1 (over 4096 bytes).
///
/// # Safety
/// `va` is this image's own mapped, device-filled frame.
unsafe fn varied(va: u64, which: &str) {
    let mut all_zero = true;
    let mut all_same = true;
    // SAFETY: the frame is this image's own RW mapping, fully written
    // by the device (the used-ring count was checked).
    let first = unsafe { core::ptr::read_volatile(va as *const u8) };
    for i in 0..DRAW {
        // SAFETY: as above; i < DRAW = the frame's size.
        let b = unsafe { core::ptr::read_volatile((va + i) as *const u8) };
        if b != 0 {
            all_zero = false;
        }
        if b != first {
            all_same = false;
        }
        if !all_zero && !all_same {
            break;
        }
    }
    if all_zero {
        log_line(|o| {
            o.str("rngtest: draw ");
            o.str(which);
            o.str(" is ALL ZERO — the device never wrote the frame");
        });
        fail(EXIT_NOT_RANDOM, "a draw came back all-zero");
    }
    if all_same {
        log_line(|o| {
            o.str("rngtest: draw ");
            o.str(which);
            o.str(" is one repeated byte — a stuck entropy source");
        });
        fail(EXIT_NOT_RANDOM, "a draw came back constant");
    }
}

/// The client entry: the spawn protocol's first thread lands here at
/// ring 3 with slot 0 holding the endpoint's call side.
///
/// # Safety
/// As the driver's `_start`: ring 3, derived stack top, kernel-loaded
/// address space, one grant in slot 0. The body is ABI v1 wrappers
/// over this image's own statics, stack, and allocated windows.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics, stack, and granted/allocated windows.
    // Single-threaded, no aliases.
    unsafe {
        log("rngtest: entropy-service client starting (two draws, variance-checked)");

        // 1. Two SEPARATE owned frames — the two draws must not share
        //    a page, or "they differ" would prove nothing about the
        //    source (only that the second overwrote the first).
        let a = draw_frame(SLOT_BUF_A, SLOT_LENT_A);
        let b = draw_frame(SLOT_BUF_B, SLOT_LENT_B);

        // 2. Two draws through the service. Each LENDS its frame; the
        //    device DMAs entropy straight into THIS process's page.
        draw(SLOT_LENT_A, "A");
        varied(a, "A");
        draw(SLOT_LENT_B, "B");
        varied(b, "B");

        // 3. The cross-draw check: two independent draws of 4096 bytes
        //    are equal only if the source loops or caches.
        let mut same = true;
        for i in 0..DRAW {
            if core::ptr::read_volatile((a + i) as *const u8)
                != core::ptr::read_volatile((b + i) as *const u8)
            {
                same = false;
                break;
            }
        }
        if same {
            log("rngtest: the two draws are byte-identical — the source is not random");
            fail(EXIT_NOT_RANDOM, "two independent draws came back identical");
        }
        // An honest fingerprint of each draw (first four bytes) so the
        // serial log shows DIFFERENT values every boot — visible
        // evidence, not a claim.
        log_line(|o| {
            o.str("rngtest: draw A ");
            for i in 0..4u64 {
                o.hex2(core::ptr::read_volatile((a + i) as *const u8));
            }
            o.str("… vs draw B ");
            for i in 0..4u64 {
                o.hex2(core::ptr::read_volatile((b + i) as *const u8));
            }
            o.str("… — both varied, and different");
        });

        // 4. The poison: rngd replies with its completion count, then
        //    exits by its own hand. Two draws → two interrupt-served
        //    completions; anything else means a completion was faked
        //    or lost.
        let (st, completions) = request(RNG_OP_SHUTDOWN, 0, CAP_NONE, EXIT_POISON);
        if st != RNG_S_OK {
            fail(EXIT_POISON, "the shutdown reply carried an error status");
        }
        if completions != 2 {
            log_line(|o| {
                o.str("rngtest: rngd reported ");
                o.u64(completions);
                o.str(" completions, expected 2");
            });
            fail(EXIT_POISON, "the driver's completion count is not two");
        }
        log(
            "rngtest: PASS — two device-filled draws, both varied, mutually different, 2 interrupt-served completions",
        );
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
        #[allow(unreachable_code)]
        loop {
            core::hint::spin_loop();
        }
    }
}
