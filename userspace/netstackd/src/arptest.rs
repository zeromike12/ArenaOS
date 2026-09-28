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
//! 4. **The stack survives its driver being killed.** After phase 1
//!    this client signals the suite, which destroys netd and lets the
//!    supervisor restart it, and then wakes the client. The next
//!    resolve is a deliberate cache MISS, so it must reach the wire
//!    through an instance that did not exist when the stack started.
//!    Re-resolving the cached gateway would have proven nothing.
//!
//! 5. **IPv4 and ICMP ride the same boundary.** A ping to the gateway
//!    is answered and timed on the monotonic clock; a ping to an
//!    address nothing answers fails as UNREACHABLE (no host) rather
//!    than NO_REPLY (a silent host), which is the layering made
//!    visible; and the demultiplexer can say how many frames of each
//!    protocol it sorted and how many it dropped.
//!
//! Exit codes: 42 verified, 60..71 typed failures, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

const SLOT_EP: u64 = 0;
/// The handshake with the suite (M7.1b): this client tells the suite
/// when it has finished phase 1, then BLOCKS until the suite says go
/// — so the driver is killed at a moment both sides agree on, rather
/// than at one the harness guessed with a sleep.
///
/// TWO notifications, with rights that make the obvious mistake
/// impossible. The first version used one for both directions, and
/// the client's own wait swallowed the badge it had just sent before
/// the suite could see it — exactly the bug faultd hit in M6.5 and
/// exactly the fix: this one is WRITE-only, the other READ-only.
const SLOT_SIGNAL: u64 = 1;
/// The suite's go-ahead. READ-only: nothing here can forge it.
const SLOT_GO: u64 = 2;
/// "Phase 1 is done — kill the driver now."
const BADGE_PHASE1: u64 = 1 << 16;
/// "The driver is dead and restarted — carry on."
const BADGE_GO: u64 = 1 << 17;

/// An address slirp also answers for (its DNS server), used AFTER the
/// restart so the lookup is a genuine cache MISS and has to reach the
/// wire through the new driver instance. Re-resolving the gateway
/// would be served from cache and would prove nothing.
const SLIRP_DNS_IP: [u8; 4] = [10, 0, 2, 3];

const EXIT_CALL: u64 = 60;
const EXIT_RESOLVE: u64 = 61;
const EXIT_ZERO_MAC: u64 = 62;
const EXIT_CACHE_MISMATCH: u64 = 63;
const EXIT_CACHE_WIRE: u64 = 64;
const EXIT_NOT_UNREACHABLE: u64 = 65;
const EXIT_STATS: u64 = 66;
/// The stack could not resolve after the driver was restarted under
/// it — the re-establish path failed.
const EXIT_AFTER_RESTART: u64 = 67;
const EXIT_PING: u64 = 68;
const EXIT_PING_RTT: u64 = 69;
const EXIT_PING_LAYER: u64 = 70;
const EXIT_DEMUX: u64 = 71;
const EXIT_BIND: u64 = 72;
const EXIT_BIND_TWICE: u64 = 73;
const EXIT_FORGED: u64 = 74;
const EXIT_UDP_SEND: u64 = 75;
const EXIT_UDP_RECV: u64 = 76;
const EXIT_DNS_MISMATCH: u64 = 77;
const EXIT_DNS_CHUNK: u64 = 78;
const EXIT_DNS_LOOKUP: u64 = 79;

/// The transaction id this client puts in its query and demands back.
const DNS_TXID: u16 = 0xA7E5;

/// Lay out a minimal DNS query for "example.com" A IN. Returns its
/// length.
fn build_dns_query(out: &mut [u8]) -> usize {
    out[0] = (DNS_TXID >> 8) as u8;
    out[1] = (DNS_TXID & 0xFF) as u8;
    out[2] = 0x01; // recursion desired
    out[3] = 0x00;
    out[4] = 0;
    out[5] = 1; // one question
    for b in out.iter_mut().take(12).skip(6) {
        *b = 0;
    }
    let mut i = 12;
    for label in [b"example".as_slice(), b"com".as_slice()] {
        out[i] = label.len() as u8;
        i += 1;
        out[i..i + label.len()].copy_from_slice(label);
        i += label.len();
    }
    out[i] = 0; // root label
    i += 1;
    out[i] = 0;
    out[i + 1] = 1; // QTYPE A
    out[i + 2] = 0;
    out[i + 3] = 1; // QCLASS IN
    i + 4
}

/// An address on the slirp network that nothing answers for. slirp
/// replies for its own gateway and DNS; .77 is simply nobody.
const SILENT_IP: [u8; 4] = [10, 0, 2, 77];

fn ip_word(ip: [u8; 4]) -> u64 {
    (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
}

/// A call carrying the inline message (IN and OUT — the kernel
/// snapshots it as the request and overwrites it with the reply).
///
/// # Safety
/// The endpoint cap is the granted slot 0; single-threaded.
unsafe fn call_msg(op: u64, w0: u64, msg: &mut [u8; MSG_BYTES]) -> (u64, u64) {
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
            msg.as_mut_ptr() as u64,
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

        // ---- 4: the driver is killed under us, and the stack copes ----
        // Tell the suite we are at the agreed point, then wait. The
        // suite destroys netd, lets the supervisor restart it, and
        // wakes us.
        log("arptest: phase 1 complete — signalling the suite to kill the driver");
        if syscall2(SYS_NOTIFY, SLOT_SIGNAL, BADGE_PHASE1) < 0 {
            fail(EXIT_CALL, "the phase-1 signal was refused");
        }
        loop {
            let b = syscall1(SYS_WAIT, SLOT_GO);
            if b < 0 {
                fail(EXIT_CALL, "waiting for the suite's go-ahead failed");
            }
            if b as u64 & BADGE_GO != 0 {
                break;
            }
        }
        log(
            "arptest: the driver has been killed and restarted — resolving through the NEW instance",
        );

        // A cache MISS on purpose: this must reach the wire, which
        // means it must go through a driver that did not exist when
        // the stack started.
        let (status, packed) = call(ARP_OP_RESOLVE, ip_word(SLIRP_DNS_IP));
        if status != ARP_S_OK {
            log_line(|o| {
                o.str("arptest: post-restart RESOLVE status ");
                o.i64(status as i64);
            });
            fail(
                EXIT_AFTER_RESTART,
                "the stack could not resolve through the restarted driver",
            );
        }
        if packed == 0 {
            fail(
                EXIT_AFTER_RESTART,
                "the post-restart resolve returned an all-zero MAC",
            );
        }
        log_line(|o| {
            o.str("arptest: PASS — 10.0.2.3 resolved to ");
            for i in 0..6u64 {
                if i > 0 {
                    o.str(":");
                }
                o.hex2(((packed >> (8 * i)) & 0xFF) as u8);
            }
            o.str(" through a driver that was RESTARTED under the stack");
        });

        // ---- 5: IPv4 + ICMP, on top of the same boundary (M7.2) ----
        // The gateway is already in the ARP cache, so this exercises
        // the IP and ICMP layers rather than re-proving resolution.
        let (status, rtt) = call(ICMP_OP_PING, ip_word(SLIRP_GATEWAY_IP));
        if status != ARP_S_OK {
            log_line(|o| {
                o.str("arptest: PING status ");
                o.i64(status as i64);
            });
            fail(EXIT_PING, "the gateway did not answer an ICMP echo");
        }
        if rtt == 0 || rtt > 2_000_000 {
            log_line(|o| {
                o.str("arptest: implausible round-trip time ");
                o.u64(rtt);
                o.str("us");
            });
            fail(
                EXIT_PING_RTT,
                "the reported round-trip time is not plausible",
            );
        }
        log_line(|o| {
            o.str("arptest: PASS — ICMP echo to 10.0.2.2 answered in ");
            o.u64(rtt);
            o.str("us (measured on the monotonic clock, identifier and sequence matched)");
        });

        // A ping to an address nothing answers must fail at the ARP
        // layer — UNREACHABLE, not NO_REPLY. The distinction is the
        // layering made visible: there is no host to send an echo to.
        let (status, _) = call(ICMP_OP_PING, ip_word(SILENT_IP));
        if status != ARP_S_UNREACHABLE {
            log_line(|o| {
                o.str("arptest: pinging an unresolvable address returned ");
                o.i64(status as i64);
            });
            fail(
                EXIT_PING_LAYER,
                "a ping to an unresolvable address did not fail at the ARP layer",
            );
        }
        log(
            "arptest: PASS — pinging an unresolvable address failed as UNREACHABLE (no host), not NO_REPLY (a silent host) — the layers report distinctly",
        );

        // The demultiplexer's own account: it must have seen both
        // kinds of frame, and it must be able to say what it dropped.
        let (status, packed) = call(ICMP_OP_RXSTATS, 0);
        if status != ARP_S_OK {
            fail(
                EXIT_STATS,
                "the stack would not report its receive counters",
            );
        }
        let (frames, arp_n, ipv4_n, dropped) = (
            packed & 0xFFFF,
            (packed >> 16) & 0xFFFF,
            (packed >> 32) & 0xFFFF,
            (packed >> 48) & 0xFFFF,
        );
        if arp_n == 0 || ipv4_n == 0 {
            log_line(|o| {
                o.str("arptest: demux saw ");
                o.u64(arp_n);
                o.str(" ARP and ");
                o.u64(ipv4_n);
                o.str(" IPv4 frames");
            });
            fail(
                EXIT_DEMUX,
                "the demultiplexer did not see both protocols (it cannot have dispatched them)",
            );
        }
        log_line(|o| {
            o.str("arptest: PASS — the demultiplexer sorted ");
            o.u64(frames);
            o.str(" frame(s): ");
            o.u64(arp_n);
            o.str(" ARP, ");
            o.u64(ipv4_n);
            o.str(" IPv4, ");
            o.u64(dropped);
            o.str(" dropped and counted");
        });

        // ---- 6: UDP, with a real server at the other end (M7.4) ----
        let (status, handle) = call(UDP_OP_BIND, 5353);
        if status != ARP_S_OK || handle == 0 {
            log_line(|o| {
                o.str("arptest: BIND status ");
                o.i64(status as i64);
            });
            fail(EXIT_BIND, "binding a UDP port failed");
        }
        // Binding the same port again must be refused: a namespace
        // rule, separate from the question of authority.
        let (status, _) = call(UDP_OP_BIND, 5353);
        if status != UDP_S_IN_USE {
            fail(EXIT_BIND_TWICE, "the same port was bound twice");
        }
        // A FORGED handle must be refused. Possession of the real one
        // is the authority, so a handle nobody issued is worth exactly
        // nothing — this is the check that makes the token meaningful.
        let (status, _) = call_msg(
            UDP_OP_SEND,
            handle ^ 0x5555_5555_5555_5555,
            &mut [0u8; MSG_BYTES],
        );
        if status != UDP_S_BAD_HANDLE {
            log_line(|o| {
                o.str("arptest: a FORGED handle was answered with status ");
                o.i64(status as i64);
            });
            fail(EXIT_FORGED, "a forged UDP handle was accepted");
        }
        log(
            "arptest: PASS — a second bind of the same port was refused, and a FORGED handle bought nothing",
        );

        // A real DNS query to slirp's resolver: 12-byte header, one
        // question for example.com, type A, class IN.
        let mut msg = [0u8; MSG_BYTES];
        msg[0..4].copy_from_slice(&SLIRP_DNS_IP);
        msg[4] = 0;
        msg[5] = 53;
        let q = build_dns_query(&mut msg[8..]);
        msg[6] = (q >> 8) as u8;
        msg[7] = (q & 0xFF) as u8;
        let (status, sent) = call_msg(UDP_OP_SEND, handle, &mut msg);
        if status != ARP_S_OK || sent as usize != q {
            log_line(|o| {
                o.str("arptest: UDP SEND status ");
                o.i64(status as i64);
            });
            fail(EXIT_UDP_SEND, "the DNS query did not go out");
        }

        let mut rx = [0u8; MSG_BYTES];
        for (b, slot) in rx.iter_mut().enumerate().take(8) {
            *slot = ((2_000_000u64 >> (8 * b)) & 0xFF) as u8;
        }
        let (status, total) = call_msg(UDP_OP_RECV, handle, &mut rx);
        if status != ARP_S_OK {
            log_line(|o| {
                o.str("arptest: UDP RECV status ");
                o.i64(status as i64);
            });
            fail(EXIT_UDP_RECV, "no DNS response came back");
        }
        // The transaction id must be the one we sent, and the response
        // bit must be set: a datagram that merely arrived proves
        // nothing about whose answer it is.
        let src_port = ((rx[4] as u16) << 8) | rx[5] as u16;
        let txid = ((rx[8] as u16) << 8) | rx[9] as u16;
        let flags = ((rx[10] as u16) << 8) | rx[11] as u16;
        if txid != DNS_TXID || flags & 0x8000 == 0 || src_port != 53 {
            log_line(|o| {
                o.str("arptest: DNS reply txid ");
                o.u64(txid as u64);
                o.str(" flags ");
                o.hex(flags as u64);
                o.str(" from port ");
                o.u64(src_port as u64);
            });
            fail(
                EXIT_DNS_MISMATCH,
                "the DNS response does not answer our query",
            );
        }
        log_line(|o| {
            o.str("arptest: PASS — UDP round trip to 10.0.2.3:53, ");
            o.u64(total);
            o.str("-byte response with our transaction id ");
            o.hex(txid as u64);
            o.str(" and the response bit set");
        });
        // A real DNS answer must cross the UDP service's inline
        // boundary; drain the rest using the SAME bearer handle.
        if total <= UDP_INLINE as u64 || total as usize > 470 {
            fail(
                EXIT_DNS_CHUNK,
                "the real DNS answer did not exercise chunked UDP receive",
            );
        }
        let mut probe = [0u8; MSG_BYTES];
        probe[..2].copy_from_slice(&(UDP_INLINE as u16).to_be_bytes());
        if call_msg(UDP_OP_RECV_CHUNK, handle ^ 0x1000, &mut probe).0 != UDP_S_BAD_HANDLE {
            fail(
                EXIT_FORGED,
                "a forged handle read the staged UDP continuation",
            );
        }
        probe[..2].copy_from_slice(&0u16.to_be_bytes());
        if call_msg(UDP_OP_RECV_CHUNK, handle, &mut probe).0 != ARP_S_BAD_OP {
            fail(
                EXIT_DNS_CHUNK,
                "an invalid UDP continuation offset was accepted",
            );
        }
        let mut full = [0u8; 470];
        full[..UDP_INLINE].copy_from_slice(&rx[8..]);
        let mut off = UDP_INLINE;
        while off < total as usize {
            let mut chunk = [0u8; MSG_BYTES];
            chunk[..2].copy_from_slice(&(off as u16).to_be_bytes());
            let (status, n) = call_msg(UDP_OP_RECV_CHUNK, handle, &mut chunk);
            if status != ARP_S_OK
                || n == 0
                || n as usize > MSG_BYTES
                || off + n as usize > total as usize
            {
                fail(
                    EXIT_DNS_CHUNK,
                    "UDP continuation was missing or out of bounds",
                );
            }
            full[off..off + n as usize].copy_from_slice(&chunk[..n as usize]);
            off += n as usize;
        }
        // The last answer's RDATA is in the tail of slirp's A reply.
        // Reassembly must preserve real bytes rather than padding.
        if full[total as usize - 4..total as usize]
            .iter()
            .all(|&b| b == 0)
        {
            fail(
                EXIT_DNS_CHUNK,
                "the continued DNS answer ended in zero padding",
            );
        }
        log_line(|o| {
            o.str("arptest: PASS — UDP answer drained across IPC messages: ");
            o.u64(off as u64);
            o.str("/ ");
            o.u64(total);
            o.str(" bytes, the tail contains answer data");
        });
        if call(UDP_OP_CLOSE, handle).0 != ARP_S_OK {
            fail(EXIT_BIND, "closing the binding failed");
        }

        // Revocation means an old handle must NOT resurrect when the
        // table slot or even the same port is bound again.
        let (status, new_handle) = call(UDP_OP_BIND, 5353);
        if status != ARP_S_OK || new_handle == 0 || new_handle == handle {
            fail(EXIT_BIND, "rebind did not issue fresh authority");
        }
        if call(UDP_OP_CLOSE, handle).0 != UDP_S_BAD_HANDLE {
            fail(EXIT_FORGED, "the closed bearer handle revived after rebind");
        }
        if call(UDP_OP_CLOSE, new_handle).0 != ARP_S_OK {
            fail(EXIT_BIND, "closing the new binding failed");
        }
        log(
            "arptest: PASS — closing and rebinding rotated the bearer; the old handle stayed revoked",
        );

        // Now ask the STACK to resolve, rather than hand-parsing an
        // opaque datagram as the M7.4 transport probe above did.
        let mut lookup = [0u8; MSG_BYTES];
        lookup[..11].copy_from_slice(b"example.com");
        let (status, addr) = call_msg(DNS_OP_LOOKUP, 11, &mut lookup);
        if status != ARP_S_OK || addr & 0xffff_ffff == 0 {
            log_line(|o| {
                o.str("arptest: DNS LOOKUP status ");
                o.i64(status as i64);
            });
            fail(EXIT_DNS_LOOKUP, "the resolver failed to parse an A answer");
        }
        let mut bad_name = [0u8; MSG_BYTES];
        bad_name[..6].copy_from_slice(b"a..b.c");
        let (status_bad, _) = call_msg(DNS_OP_LOOKUP, 6, &mut bad_name);
        if status_bad != DNS_S_BAD_NAME {
            fail(EXIT_DNS_LOOKUP, "a malformed name was accepted");
        }
        log_line(|o| {
            o.str("arptest: PASS — DNS resolver returned example.com A = ");
            for i in 0..4 {
                if i != 0 {
                    o.str(".");
                }
                o.u64((addr >> (8 * i)) & 0xff);
            }
            o.str("; a malformed name was refused BEFORE sending");
        });

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
