//! `arptest` — the first protocol, proven on the wire (M7.1,
//! ADR-0030). Spawn-registry image 18, one grant: slot 0 =
//! `Endpoint` (WRITE — the call side of netstackd's service).
//!
//! What it proves:
//!
//! 1. **Resolution is real.** Ask the stack for the slirp gateway's
//!    MAC. The answer must come back non-zero, and the m7 suite
//!    cross-checks on the kernel side that a frame actually left and
//!    an interrupt actually arrived — so this is a wire round trip,
//!    not a lookup table.
//! 2. **The cache is real.** Ask again. The answer must be identical
//!    AND the stack's count of ARP requests PUT ON THE WIRE must not
//!    move. A cache that still sends the packet is not a cache; the
//!    only honest proof is that the wire stayed quiet.
//! 3. **An unreachable address is reported, not invented.** Ask for
//!    an address nothing will answer for. It must come back
//!    `ARP_S_UNREACHABLE` after real retries and real deadlines — and
//!    crucially, this must TERMINATE, which is the whole reason the
//!    timer facility was built before any protocol existed. Before
//!    M7.0 this call would have parked the stack forever.
//!
//! Exit codes: 42 verified, 60..66 typed failures, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_EP: u64 = 0;

const EXIT_CALL: u64 = 60;
const EXIT_RESOLVE: u64 = 61;
const EXIT_ZERO_MAC: u64 = 62;
const EXIT_CACHE_MISMATCH: u64 = 63;
const EXIT_CACHE_WIRE: u64 = 64;
const EXIT_NOT_UNREACHABLE: u64 = 65;
const EXIT_STATS: u64 = 66;

/// An address on the slirp network that nothing answers for. slirp
/// replies for its own gateway and DNS; .77 is simply nobody.
const SILENT_IP: [u8; 4] = [10, 0, 2, 77];

fn ip_word(ip: [u8; 4]) -> u64 {
    (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
}

/// # Safety
/// The endpoint cap is the granted slot 0; single-threaded.
unsafe fn call(op: u64, w0: u64) -> (u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            w0,
            op,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            0,
        )
    };
    if r < 0 {
        log_line(|o| {
            o.str("arptest: IPC_CALL returned ");
            o.i64(r);
        });
        fail(EXIT_CALL, "the call to the stack was refused");
    }
    (reply[0], reply[1])
}

/// # Safety
/// Entered by the spawn protocol exactly as every other image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and stack. Single-threaded, no aliases.
    unsafe {
        log("arptest: asking the stack to resolve an address on the real wire");

        // ---- 1: a real resolution ----
        let (status, packed) = call(ARP_OP_RESOLVE, ip_word(SLIRP_GATEWAY_IP));
        if status != ARP_S_OK {
            log_line(|o| {
                o.str("arptest: RESOLVE status ");
                o.i64(status as i64);
            });
            fail(EXIT_RESOLVE, "the gateway did not resolve");
        }
        if packed == 0 {
            fail(EXIT_ZERO_MAC, "the stack returned an all-zero MAC");
        }
        let first = packed;
        log_line(|o| {
            o.str("arptest: PASS — 10.0.2.2 resolved to ");
            for i in 0..6u64 {
                if i > 0 {
                    o.str(":");
                }
                o.hex2(((packed >> (8 * i)) & 0xFF) as u8);
            }
            o.str(" through a real ARP exchange");
        });

        let (status, _hits, wire_before) = stats();
        if status != ARP_S_OK {
            fail(EXIT_STATS, "the stack would not report its counters");
        }

        // ---- 2: the cache must keep the wire quiet ----
        let (status, packed) = call(ARP_OP_RESOLVE, ip_word(SLIRP_GATEWAY_IP));
        if status != ARP_S_OK {
            fail(EXIT_RESOLVE, "the second resolve failed");
        }
        if packed != first {
            fail(EXIT_CACHE_MISMATCH, "the cache returned a different MAC");
        }
        let (_, hits_after, wire_after) = stats();
        if wire_after != wire_before {
            log_line(|o| {
                o.str("arptest: wire requests went ");
                o.u64(wire_before);
                o.str(" → ");
                o.u64(wire_after);
                o.str(" on a cached lookup");
            });
            fail(
                EXIT_CACHE_WIRE,
                "a cached answer still put a request on the wire",
            );
        }
        if hits_after == 0 {
            fail(EXIT_CACHE_WIRE, "the stack counted no cache hit");
        }
        log_line(|o| {
            o.str("arptest: PASS — the second lookup was a cache HIT (");
            o.u64(hits_after);
            o.str(" hit(s), wire requests still ");
            o.u64(wire_after);
            o.str(")");
        });

        // ---- 3: silence is reported, and TERMINATES ----
        let (status, _) = call(ARP_OP_RESOLVE, ip_word(SILENT_IP));
        if status != ARP_S_UNREACHABLE {
            log_line(|o| {
                o.str("arptest: resolving a silent address returned ");
                o.i64(status as i64);
            });
            fail(
                EXIT_NOT_UNREACHABLE,
                "an address nobody answers for did not come back unreachable",
            );
        }
        log(
            "arptest: PASS — a silent address came back UNREACHABLE after real deadlines (before M7.0 this call could never have returned)",
        );

        let (status, wire_total, hits_total) = shutdown();
        if status != ARP_S_OK {
            fail(EXIT_STATS, "the poison shutdown was refused");
        }
        log_line(|o| {
            o.str("arptest: done — the stack put ");
            o.u64(wire_total);
            o.str(" request(s) on the wire and served ");
            o.u64(hits_total);
            o.str(" from cache");
        });
        syscall1(SYS_THREAD_EXIT, EXIT_OK);
    }
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

/// (status, hits, wire_requests) — the pair arrives packed in one
/// reply word (hits low 32, wire high 32), because an IPC v1.1 reply
/// carries exactly one payload word beside its status.
///
/// # Safety
/// As `call`.
unsafe fn stats() -> (u64, u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            0,
            ARP_OP_STATS,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            0,
        )
    };
    if r < 0 {
        fail(EXIT_CALL, "the stats call was refused");
    }
    (reply[0], reply[1] & 0xFFFF_FFFF, reply[1] >> 32)
}

/// # Safety
/// As `call`.
unsafe fn shutdown() -> (u64, u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_EP,
            0,
            ARP_OP_SHUTDOWN,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            0,
        )
    };
    if r < 0 {
        fail(EXIT_CALL, "the shutdown call was refused");
    }
    (reply[0], reply[1] >> 32, reply[1] & 0xFFFF_FFFF)
}

fn log(s: &str) {
    log_line(|o| o.str(s));
}

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("arptest: FAIL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges; the code is the m7 contract.
    unsafe { syscall1(SYS_THREAD_EXIT, code) };
    #[allow(unreachable_code)]
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    fail(EXIT_PANIC, "panic")
}
