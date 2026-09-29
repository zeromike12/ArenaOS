//! ArenaOS network stack service — `netstackd` (M7.1, ADR-0030).
//! Spawn-registry image 17, with kernel-literal grants:
//!
//! - slot 0: `Endpoint` (WRITE — the CALL side of netd's service; the
//!   stack is a client of the driver, never the other way round),
//! - slot 1: `Endpoint` (READ — its own serve side, where clients ask
//!   it to resolve addresses),
//! - slot 2: `Notification` (READ|WRITE — backoff timers while a dead
//!   driver is being restarted; M7.1b),
//!
//! and fills slots 3..5 itself with its transmit frame and the lend
//! copies of it.
//!
//! **Where the line is drawn.** netd owns the device: virtqueues,
//! interrupts, the MAC in config space, and raw Ethernet frames in
//! and out. It does not parse an ethertype or hold an ARP entry.
//! Everything above L2 lives here. That split is not tidiness — a
//! driver's job is to survive its device and be restartable (M6.5),
//! while a stack's job is to hold state across time, and a stack
//! living inside its driver would lose every connection each time the
//! NIC wedged.
//!
//! **What it does.** ARP over IPv4 with a TTL cache (M7.1), and since
//! M7.2 a receive DEMULTIPLEXER with IPv4 and ICMP echo on top of it:
//! `ping` resolves the address if it has to, sends a real echo
//! request, and times the answer with the monotonic clock.
//!
//! The demultiplexer is the structural change. With one protocol the
//! receive path could live inside `resolve`, which read frames itself
//! and discarded anything that was not an ARP reply. With two that is
//! wrong: an echo reply arriving while the stack happened to be
//! resolving would be thrown away, and the ping waiting for it would
//! time out for no reason at all. Recognising a frame is now one job
//! in one place, and operations wait on state the demultiplexer
//! updates.
//!
//! IPv4 here is deliberately the smallest thing that deserves the
//! name: fixed 20-byte header, no options, no fragmentation, no
//! routing (every destination is on-link and resolved with ARP). What
//! IS implemented is done properly — both checksums are computed on
//! send and VERIFIED on receive, an echo reply must match the
//! identifier AND sequence that went out, and anything else is
//! counted and dropped rather than believed.
//!
//! **Why the deadline lives in netd.** A lost ARP reply must not park
//! this service forever. It cannot bound its own wait, because a
//! thread blocked in `SYS_IPC_CALL` cannot observe its own timer
//! (ADR-0029's erratum), so the bound is passed DOWN: `NET_OP_RECV`
//! takes a timeout and netd arms the timer on its own notification.
//! This is the first production use of the M7.0 facility and the
//! reason it was built before any protocol existed.
//!
//! **Aging is by clock, not by timer.** Entries carry an expiry and
//! are checked when looked at. A timer per entry would buy nothing:
//! nothing has to HAPPEN when an ARP entry expires, and the only
//! moment staleness matters is the moment somebody asks. Timers are
//! for things that must act on their own; this is not one.
//!
//! Exit codes: 42 clean shutdown, 80..89 stage failures, 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;
mod dns;
mod tcp;

const SLOT_NETD: u64 = 0;
const SLOT_EP: u64 = 1;
/// A notification of its own (M7.1b): what the stack backs off on
/// while the supervisor brings a dead driver back. Waiting needs
/// something to wait ON, and spinning on the clock is the polling
/// Phase 7 forbids. GRANTED, so it sits with the other grants at the
/// bottom of the space — the slots this program fills itself start
/// after them.
const SLOT_NOTIF: u64 = 2;
/// The entropy service's call side (M7.4). netstackd draws its UDP
/// handle pool from rngd at startup: a handle is authority by
/// possession, so a guessable one would be authority by arithmetic.
const SLOT_RNG: u64 = 3;
const SLOT_TX: u64 = 4;
/// The MASTER lend copy, taken once before the frame is mapped.
///
/// `SYS_MAP_MEMORY` CONSUMES the cap it maps (ADR-0021: ownership
/// moves into the address space), so after mapping there is no
/// original left to copy from. rngd learned this once and netstackd
/// promptly learned it again: a per-send copy taken from slot 2 after
/// the map is a copy of nothing. The master is lent, never mapped,
/// and every send copies from IT — because each send CONSUMES its
/// copy on the way to the driver.
const SLOT_TX_MASTER: u64 = 5;
/// The per-send lend copy, consumed by the call that carries it.
const SLOT_TX_LENT: u64 = 6;

/// Badge for the re-attach backoff timer.
const BADGE_BACKOFF: u64 = 1 << 16;
/// How long to wait between attempts to find the driver alive again.
/// The supervisor restarts from another thread; this is patience, not
/// a guess about how long a spawn takes.
const REATTACH_BACKOFF_US: u64 = 20_000;
/// How many times to look for the driver before giving up on it.
const REATTACH_TRIES: u32 = 25;

const EXIT_SETUP: u64 = 80;
const EXIT_MAC: u64 = 81;
const EXIT_RECV: u64 = 85;
const EXIT_REPLY: u64 = 88;
/// The receive parser accepted something its specification forbids.
const EXIT_SELFTEST: u64 = 90;

/// Cached entries. Small on purpose: this is a fixture-scale stack,
/// and a bound that fits on one screen is a bound that gets checked.
const CACHE_LEN: usize = 8;
/// How long a resolved entry stays good. Short by real-network
/// standards (minutes are typical) because nothing here benefits from
/// a long one and a short TTL keeps the aging path exercised.
const TTL_US: u64 = 30_000_000;
/// How long to wait for one ARP reply before retrying.
const REPLY_TIMEOUT_US: u64 = 250_000;
/// How many times to ask before giving up. A request is idempotent (a
/// broadcast query), so retrying it is safe — unlike a datagram send,
/// which must never be retried blind (ROADMAP Phase 7, decision 3).
const ARP_TRIES: u32 = 3;

const ARP_FRAME_LEN: u64 = 42;
const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_IPV4: u16 = 0x0800;
const ARP_OP_REQUEST: u16 = 1;
const ARP_OP_REPLY: u16 = 2;

// ---- IPv4 + ICMP (M7.2) -----------------------------------------------------
//
// Deliberately the smallest thing that can be called IPv4: a fixed
// 20-byte header, no options, no fragmentation, no routing (every
// destination is treated as on-link and resolved with ARP), and TTL
// 64 because that is what everything else picks. What IS here is
// done properly — the header checksum is computed on send and
// VERIFIED on receive, and a frame that fails it is counted and
// dropped rather than trusted.

/// Ethernet header length: dst(6) + src(6) + ethertype(2).
const ETH_HDR: usize = 14;
/// IPv4 header without options.
const IP_HDR: usize = 20;
/// ICMP echo header: type, code, checksum, identifier, sequence.
const ICMP_HDR: usize = 8;
/// Bytes of payload carried in an echo request. Small on purpose: the
/// whole frame must fit netd's 64-byte inline reply limit, and a
/// 50-byte frame comes back padded to the 60-byte Ethernet minimum.
const PING_PAYLOAD: usize = 200;
/// Largest frame this stack will reassemble. One Ethernet MTU is the
/// eventual answer; 512 is what the current receive path and this
/// service's budget support, and it is stated rather than implied.
const FRAME_MAX: usize = 512;
const PING_FRAME_LEN: u64 = (ETH_HDR + IP_HDR + ICMP_HDR + PING_PAYLOAD) as u64;

const IP_PROTO_ICMP: u8 = 1;
/// Bindings this stack will hold at once. Small and stated; the
/// handle pool is drawn to match.
const UDP_BINDINGS: usize = 8;
/// The UDP header: source port, destination port, length, checksum.
const UDP_HDR: usize = 8;
/// Bounded by the stack's frame reassembly limit, not the IPC message.
const UDP_MAX: usize = FRAME_MAX - ETH_HDR - IP_HDR - UDP_HDR;
/// Internal DNS transaction port; never offered to a UDP BIND caller.
const DNS_PORT: u16 = 5354;
const DNS_TIMEOUT_US: u64 = 2_000_000;

const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;
/// Time to live. Nothing here routes, so this only has to be
/// plausible to whatever answers.
const IP_TTL: u8 = 64;
/// How long to wait for an echo reply before calling it silence.
const PING_TIMEOUT_US: u64 = 500_000;
/// How many echoes to send before giving up. ICMP is unreliable and a
/// single lost packet is not an unreachable host — but note this is
/// NOT the same licence as ARP's retry: an echo is idempotent because
/// it is a diagnostic with a sequence number, not because retrying
/// datagrams is generally safe.
const PING_TRIES: u32 = 3;

#[derive(Clone, Copy)]
struct Entry {
    live: bool,
    ip: [u8; 4],
    mac: [u8; 6],
    expires_us: u64,
}

/// One UDP binding: a port, the handle that authorises its use, and a
/// single-slot inbox for the most recent datagram.
#[derive(Clone, Copy)]
struct Binding {
    live: bool,
    port: u16,
    handle: u64,
    /// The newest datagram, if one is waiting. Single-slot on purpose:
    /// UDP may drop, and a queue nobody drains is a leak that looks
    /// like a feature.
    have: bool,
    /// A delivered long datagram remains available until its last chunk.
    staged: bool,
    src_ip: [u8; 4],
    src_port: u16,
    len: usize,
    data: [u8; UDP_MAX],
}

const NO_BINDING: Binding = Binding {
    live: false,
    port: 0,
    handle: 0,
    have: false,
    staged: false,
    src_ip: [0; 4],
    src_port: 0,
    len: 0,
    data: [0; UDP_MAX],
};

// The 8 full-sized inboxes live in BSS, not on the one-page userspace
// stack. Only `_start` obtains a reference and this image has one thread.
static mut BINDINGS: [Binding; UDP_BINDINGS] = [NO_BINDING; UDP_BINDINGS];

/// One stop-and-wait TCP connection. The peer's advertised window is
/// observed before WRITE; our MSS/window stay below FRAME_MAX. The
/// pending segment is retained for sequence-identical retransmission.
#[derive(Clone, Copy)]
struct TcpConn {
    live: bool,
    handle: u64,
    ip: [u8; 4],
    mac: [u8; 6],
    src_port: u16,
    dst_port: u16,
    state: u64,
    snd_nxt: u32,
    rcv_nxt: u32,
    peer_window: u16,
    peer_fin: bool,
    pending: bool,
    pending_seq: u32,
    pending_flags: u8,
    pending_len: usize,
    pending_data: [u8; MSG_BYTES - 1],
    sent_us: u64,
    retries: u8,
    rx_len: usize,
    rx: [u8; tcp::MSS],
    retransmits: u64,
}
const NO_TCP: TcpConn = TcpConn {
    live: false,
    handle: 0,
    ip: [0; 4],
    mac: [0; 6],
    src_port: 0,
    dst_port: 0,
    state: TCP_CLOSED,
    snd_nxt: 0,
    rcv_nxt: 0,
    peer_window: 0,
    peer_fin: false,
    pending: false,
    pending_seq: 0,
    pending_flags: 0,
    pending_len: 0,
    pending_data: [0; MSG_BYTES - 1],
    sent_us: 0,
    retries: 0,
    rx_len: 0,
    rx: [0; tcp::MSS],
    retransmits: 0,
};

const EMPTY: Entry = Entry {
    live: false,
    ip: [0; 4],
    mac: [0; 6],
    expires_us: 0,
};

struct Stack {
    mac: [u8; 6],
    tx: u64,
    cache: [Entry; CACHE_LEN],
    hits: u64,
    wire_requests: u64,
    replies_seen: u64,
    timeouts: u64,
    /// Times the driver died under us and was picked back up.
    reattaches: u64,
    // ---- the receive demultiplexer's own account of itself (M7.2) ----
    rx_frames: u64,
    rx_arp: u64,
    rx_ipv4: u64,
    rx_dropped: u64,
    rx_bad_checksum: u64,
    /// Fragments seen and refused (v1 does not reassemble).
    rx_fragments: u64,
    /// Echo replies refused because they came from the wrong host.
    rx_wrong_source: u64,
    /// Continuation chunks read for frames larger than one message.
    rx_chunks: u64,
    /// Frames longer than this stack will reassemble.
    rx_oversize: u64,
    /// Echo replies whose payload did not survive reassembly.
    rx_corrupt_payload: u64,
    /// UDP datagrams delivered to a binding, and refused for want of
    /// one (M7.4).
    rx_udp: u64,
    rx_udp_unbound: u64,
    /// Handles presented that name no live binding — including forged
    /// ones, which is the number worth watching.
    bad_handles: u64,
    /// The port table, and the pool of unguessable handles it issues.
    bindings: &'static mut [Binding; UDP_BINDINGS],
    handle_pool: [u64; UDP_BINDINGS],
    issued: [bool; UDP_BINDINGS],
    have_entropy: bool,
    /// The ARP answer this stack is currently waiting for, and the
    /// answer once it lands. Held here rather than on the call stack
    /// because the demultiplexer — not the caller — is what recognises
    /// a frame now (M7.2).
    want_arp: Option<[u8; 4]>,
    got_arp: Option<[u8; 6]>,
    /// The echo this stack is waiting for: (identifier, sequence) AND
    /// the address it was sent to. The address matters: identifier and
    /// sequence alone can be satisfied by a reply from a DIFFERENT
    /// host that happens to use the same pair, which on any shared
    /// network is not a hypothetical (found in review of v0.14.0).
    want_icmp: Option<(u16, u16)>,
    want_icmp_ip: Option<[u8; 4]>,
    got_icmp: bool,
    ping_seq: u16,
    tcp: TcpConn,
}

impl Stack {
    fn lookup(&mut self, ip: [u8; 4], now: u64) -> Option<[u8; 6]> {
        for e in self.cache.iter_mut() {
            if e.live && e.ip == ip {
                if now >= e.expires_us {
                    // Aged out. Checked HERE, at the only moment
                    // staleness can matter, rather than by a timer
                    // that would have nothing to do when it fired.
                    *e = EMPTY;
                    return None;
                }
                return Some(e.mac);
            }
        }
        None
    }

    fn insert(&mut self, ip: [u8; 4], mac: [u8; 6], now: u64) {
        // Replace a matching or dead entry; otherwise take slot 0
        // (v1 has no eviction policy worth the name, and says so).
        let mut idx = 0;
        for (i, e) in self.cache.iter().enumerate() {
            if !e.live || e.ip == ip {
                idx = i;
                break;
            }
        }
        self.cache[idx] = Entry {
            live: true,
            ip,
            mac,
            expires_us: now.saturating_add(TTL_US),
        };
    }
}

/// One call down to netd. Returns (status, word1).
///
/// # Safety
/// The netd endpoint cap is the granted slot 0; `msg` is this image's
/// own buffer; single-threaded.
unsafe fn netd_call(op: u64, w0: u64, cap: u64, msg: u64) -> (u64, u64) {
    let mut reply = [0u64; 3];
    // SAFETY: wrapper contract.
    let r = unsafe {
        syscall6(
            SYS_IPC_CALL,
            SLOT_NETD,
            w0,
            op,
            cap,
            reply.as_mut_ptr() as u64,
            msg,
        )
    };
    if r < 0 {
        // STATUS_SERVICE_GONE lands here when the driver has died
        // under us (ADR-0028). v1 reports it upward as a link failure
        // rather than retrying blind: the caller decides, and a
        // re-attach (not a retry) is what a restarted netd needs —
        // ROADMAP Phase 7, decision 3, and the 7.1b proof.
        log_line(|o| {
            o.str("netstackd: the driver refused a call: ");
            o.i64(r);
        });
        return (ARP_S_LINK_DOWN, 0);
    }
    (reply[0], reply[1])
}

/// # Safety
/// Entered by the spawn protocol exactly as every other image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics, stack and mapped frame. Single-threaded.
    unsafe {
        log("netstackd: starting — ArenaOS network stack service (M7.1, ADR-0030)");

        // The transmit frame: allocate, then map. The cap copy for
        // lending is taken per-send, because the map consumes the
        // original and a lend must never outlive its request.
        let phys = syscall1(SYS_ALLOC_FRAME, SLOT_TX);
        if phys <= 0 {
            fail(EXIT_SETUP, "transmit frame allocation refused");
        }
        // The master copy FIRST — the map below consumes slot 2.
        if syscall3(SYS_CAP_COPY, SLOT_TX, SLOT_TX_MASTER, RIGHTS_ALL) < 0 {
            fail(EXIT_SETUP, "the master lend copy refused");
        }
        let win = syscall2(SYS_MAP_MEMORY, SLOT_TX, 1);
        if win <= 0 {
            fail(EXIT_SETUP, "the transmit frame self-map refused");
        }

        // Our MAC, through the driver's service boundary — the stack
        // never touches config space itself.
        let (st, packed) = netd_call(NET_OP_MAC, 0, CAP_NONE, 0);
        if st != NET_S_OK {
            fail(EXIT_MAC, "the driver would not report the device MAC");
        }
        let mut mac = [0u8; 6];
        for (i, m) in mac.iter_mut().enumerate() {
            *m = ((packed >> (8 * i)) & 0xFF) as u8;
        }
        if mac.iter().all(|&b| b == 0) {
            fail(EXIT_MAC, "the driver reported an all-zero MAC");
        }

        let bindings_ptr = &raw mut BINDINGS;
        let mut st8 = Stack {
            mac,
            tx: win as u64,
            cache: [EMPTY; CACHE_LEN],
            hits: 0,
            wire_requests: 0,
            replies_seen: 0,
            timeouts: 0,
            reattaches: 0,
            rx_frames: 0,
            rx_arp: 0,
            rx_ipv4: 0,
            rx_dropped: 0,
            rx_bad_checksum: 0,
            rx_fragments: 0,
            rx_wrong_source: 0,
            rx_chunks: 0,
            rx_oversize: 0,
            rx_corrupt_payload: 0,
            rx_udp: 0,
            rx_udp_unbound: 0,
            bad_handles: 0,
            // SAFETY: sole reference for the sole thread of this image.
            bindings: &mut *bindings_ptr,
            handle_pool: [0; UDP_BINDINGS],
            issued: [false; UDP_BINDINGS],
            have_entropy: false,
            want_arp: None,
            got_arp: None,
            want_icmp: None,
            want_icmp_ip: None,
            got_icmp: false,
            ping_seq: 0,
            tcp: NO_TCP,
        };
        draw_handles(&mut st8);
        let selftest = parser_selftest(&mut st8);
        if selftest == 7 {
            log(
                "netstackd: receive-parser self-test PASSED 7/7 — options, fragments, short frames, bad checksums and foreign echo replies all refused, and the genuine article still accepted",
            );
        } else {
            log_line(|o| {
                o.str("netstackd: receive-parser self-test FAILED (");
                o.u64(selftest as u64);
                o.str("/7) — the parser accepts something it must not");
            });
            fail(
                EXIT_SELFTEST,
                "the receive parser accepts more than its specification",
            );
        }

        let udp_checks = udp_selftest(&mut st8);
        if udp_checks != 4 {
            fail(EXIT_SELFTEST, "UDP checksum self-test failed");
        }
        log(
            "netstackd: UDP checksum self-test PASSED 4/4 — outgoing nonzero and valid, incoming valid/nonzero accepted, corruption counted, IPv4 zero-checksum accepted",
        );

        log_line(|o| {
            o.str("netstackd: ready — ARP over IPv4 for ");
            for (i, b) in mac.iter().enumerate() {
                if i > 0 {
                    o.str(":");
                }
                o.hex2(*b);
            }
            o.str(", cache ");
            o.u64(CACHE_LEN as u64);
            o.str(" entries, TTL 30s; netd stays L2");
        });

        // ADR-0039: signal startup on the SAME notification used for
        // later backoff, avoiding an unrequested fifth child grant.
        // In M7's standalone fixture nobody waits on this bit; the
        // driver's own backoff waits accept a merged badge and retry.
        let ready = syscall2(SYS_NOTIFY, SLOT_NOTIF, MGR_BADGE_STACK_READY);
        if ready != 0 {
            fail(EXIT_SETUP, "stack readiness notification refused");
        }
        serve(&mut st8);
    }
}

/// The serve loop: clients ask, the stack answers.
///
/// # Safety
/// As `_start`.
unsafe fn serve(st8: &mut Stack) -> ! {
    let mut w = [0u64; 3];
    let mut inbox = [0u8; MSG_BYTES];
    loop {
        // SAFETY: function contract.
        unsafe {
            let r = syscall3(
                SYS_IPC_RECV,
                SLOT_EP,
                w.as_mut_ptr() as u64,
                inbox.as_mut_ptr() as u64,
            );
            if r < 0 {
                log_line(|o| {
                    o.str("netstackd: IPC_RECV returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "the serve-side receive failed");
            }
            let (arg, op, landed) = (w[0], w[1], w[2]);
            if landed != CAP_NONE {
                let _ = syscall1(SYS_CAP_DESTROY, landed);
            }
            match op {
                ARP_OP_FAULT => {
                    log_line(|o| o.str("netstackd: injecting real #UD before in-flight reply"));
                    core::arch::asm!("ud2", options(noreturn));
                }
                ARP_OP_RESOLVE => {
                    let ip = [
                        (arg & 0xFF) as u8,
                        ((arg >> 8) & 0xFF) as u8,
                        ((arg >> 16) & 0xFF) as u8,
                        ((arg >> 24) & 0xFF) as u8,
                    ];
                    match resolve(st8, ip) {
                        Some(mac) => {
                            let mut packed = 0u64;
                            for (i, b) in mac.iter().enumerate() {
                                packed |= (*b as u64) << (8 * i);
                            }
                            reply(ARP_S_OK, packed, 0);
                        }
                        None => reply(ARP_S_UNREACHABLE, 0, 0),
                    }
                }
                // Two counters in ONE word: an IPC v1.1 reply carries
                // a status and a single payload word, so a service
                // returning a pair packs it (hits low, wire high).
                // The first version read reply[2] on the client side,
                // which is the LANDED CAP slot — it came back as
                // 18446744073709551615 and looked like a counter.
                ICMP_OP_PING => {
                    let ip = [
                        (arg & 0xFF) as u8,
                        ((arg >> 8) & 0xFF) as u8,
                        ((arg >> 16) & 0xFF) as u8,
                        ((arg >> 24) & 0xFF) as u8,
                    ];
                    match ping(st8, ip) {
                        Ok(rtt) => reply(ARP_S_OK, rtt, 0),
                        Err(status) => reply(status, 0, 0),
                    }
                }
                UDP_OP_BIND => {
                    let port = (arg & 0xFFFF) as u16;
                    if !st8.have_entropy {
                        // No unguessable handle to issue, so no
                        // binding. Refusing beats handing out an
                        // authority anyone can guess.
                        reply(ARP_S_LINK_DOWN, 0, 0);
                        continue;
                    }
                    if port == 0
                        || port == DNS_PORT
                        || st8.bindings.iter().any(|b| b.live && b.port == port)
                    {
                        reply(UDP_S_IN_USE, 0, 0);
                        continue;
                    }
                    match (0..UDP_BINDINGS).find(|&i| !st8.bindings[i].live) {
                        Some(i) => {
                            // A CLOSED handle must never re-authorise a
                            // later occupant of the same slot. Rotate it
                            // from rngd before every reuse; otherwise
                            // revocation is decorative (ADR-0033).
                            if st8.issued[i] && !refresh_handle(st8, i) {
                                reply(ARP_S_LINK_DOWN, 0, 0);
                                continue;
                            }
                            let handle = st8.handle_pool[i];
                            st8.issued[i] = true;
                            st8.bindings[i] = Binding {
                                live: true,
                                port,
                                handle,
                                ..NO_BINDING
                            };
                            log_line(|o| {
                                o.str("netstackd: port ");
                                o.u64(port as u64);
                                o.str(" bound — handle issued (possession is the authority; the stack cannot say WHO holds it, and does not pretend to)");
                            });
                            reply(ARP_S_OK, handle, 0);
                        }
                        None => reply(UDP_S_IN_USE, 0, 0),
                    }
                }
                UDP_OP_SEND => {
                    let Some(i) = binding_of(st8, arg) else {
                        st8.bad_handles += 1;
                        reply(UDP_S_BAD_HANDLE, 0, 0);
                        continue;
                    };
                    let dst_ip = [inbox[0], inbox[1], inbox[2], inbox[3]];
                    let dst_port = ((inbox[4] as u16) << 8) | inbox[5] as u16;
                    let plen = (((inbox[6] as u16) << 8) | inbox[7] as u16) as usize;
                    if plen > UDP_INLINE {
                        reply(ARP_S_BAD_OP, 0, 0);
                        continue;
                    }
                    let Some(mac) = resolve(st8, dst_ip) else {
                        reply(ARP_S_UNREACHABLE, 0, 0);
                        continue;
                    };
                    let src_port = st8.bindings[i].port;
                    let flen = build_udp(
                        st8,
                        mac,
                        dst_ip,
                        src_port,
                        dst_port,
                        inbox.as_ptr().add(8),
                        plen,
                    );
                    let status = transmit(st8, flen);
                    if status != NET_S_OK {
                        reply(ARP_S_LINK_DOWN, 0, 0);
                        continue;
                    }
                    reply(ARP_S_OK, plen as u64, 0);
                }
                UDP_OP_RECV => {
                    let Some(i) = binding_of(st8, arg) else {
                        st8.bad_handles += 1;
                        reply(UDP_S_BAD_HANDLE, 0, 0);
                        continue;
                    };
                    let mut timeout_us = 0u64;
                    for (b, byte) in inbox.iter().enumerate().take(8) {
                        timeout_us |= (*byte as u64) << (8 * b);
                    }
                    // Starting another receive abandons any unread
                    // continuation for this binding (bounded v1 storage).
                    st8.bindings[i].staged = false;
                    // Pump the wire until this binding has something
                    // or the deadline passes. The DEMULTIPLEXER is
                    // what decides a frame belongs here — this loop
                    // just keeps the wire moving (M7.2).
                    while !st8.bindings[i].have {
                        if !pump(st8, timeout_us) {
                            break;
                        }
                    }
                    if !st8.bindings[i].have {
                        reply(UDP_S_NO_DATA, 0, 0);
                        continue;
                    }
                    let b = st8.bindings[i];
                    st8.bindings[i].have = false;
                    let keep = core::cmp::min(b.len, UDP_INLINE);
                    st8.bindings[i].staged = b.len > UDP_INLINE;
                    let mut out = [0u8; MSG_BYTES];
                    out[..4].copy_from_slice(&b.src_ip);
                    out[4] = (b.src_port >> 8) as u8;
                    out[5] = (b.src_port & 0xFF) as u8;
                    out[6] = (keep >> 8) as u8;
                    out[7] = (keep & 0xFF) as u8;
                    out[8..8 + keep].copy_from_slice(&b.data[..keep]);
                    let rr = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        ARP_S_OK,
                        b.len as u64,
                        CAP_NONE,
                        out.as_ptr() as u64,
                    );
                    if rr < 0 {
                        fail(EXIT_REPLY, "the UDP receive reply was refused");
                    }
                }
                TCP_OP_OPEN => {
                    if arg >> 32 != 0 {
                        reply(ARP_S_BAD_OP, 0, 0);
                        continue;
                    }
                    let ip = [
                        arg as u8,
                        (arg >> 8) as u8,
                        (arg >> 16) as u8,
                        (arg >> 24) as u8,
                    ];
                    let port = u16::from_be_bytes([inbox[0], inbox[1]]);
                    match tcp_open(st8, ip, port) {
                        Ok(handle) => reply(ARP_S_OK, handle, 0),
                        Err(code) => reply(code, 0, 0),
                    }
                }
                TCP_OP_POLL => {
                    if !st8.tcp.live || st8.tcp.handle != arg {
                        reply(TCP_S_BAD_HANDLE, 0, 0);
                        continue;
                    }
                    let timeout = u64::from_le_bytes(inbox[..8].try_into().unwrap());
                    tcp_poll(st8, timeout);
                    reply(ARP_S_OK, st8.tcp.state | ((st8.tcp.rx_len as u64) << 8), 0);
                }
                TCP_OP_WRITE => {
                    if !st8.tcp.live || st8.tcp.handle != arg {
                        reply(TCP_S_BAD_HANDLE, 0, 0);
                        continue;
                    }
                    let n = inbox[0] as usize;
                    if n == 0
                        || n >= MSG_BYTES
                        || st8.tcp.state != TCP_ESTABLISHED
                        || st8.tcp.peer_fin
                        || st8.tcp.pending
                        || n > st8.tcp.peer_window as usize
                    {
                        reply(TCP_S_STATE, 0, 0);
                        continue;
                    }
                    tcp_send_new(st8, tcp::ACK | tcp::PSH, &inbox[1..1 + n]);
                    reply(ARP_S_OK, n as u64, 0);
                }
                TCP_OP_READ => {
                    if !st8.tcp.live || st8.tcp.handle != arg {
                        reply(TCP_S_BAD_HANDLE, 0, 0);
                        continue;
                    }
                    let was_full = st8.tcp.rx_len == tcp::MSS;
                    let n = core::cmp::min(st8.tcp.rx_len, MSG_BYTES);
                    let mut out = [0u8; MSG_BYTES];
                    out[..n].copy_from_slice(&st8.tcp.rx[..n]);
                    st8.tcp.rx.copy_within(n..st8.tcp.rx_len, 0);
                    st8.tcp.rx_len -= n;
                    let rr = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        ARP_S_OK,
                        n as u64,
                        CAP_NONE,
                        out.as_ptr() as u64,
                    );
                    if rr < 0 {
                        fail(EXIT_REPLY, "TCP read reply refused");
                    }
                    if was_full && n != 0 {
                        tcp_send_raw(st8, tcp::ACK, st8.tcp.snd_nxt, &[]);
                    }
                }
                TCP_OP_CLOSE => {
                    if !st8.tcp.live || st8.tcp.handle != arg {
                        reply(TCP_S_BAD_HANDLE, 0, 0);
                    } else if st8.tcp.state != TCP_ESTABLISHED || st8.tcp.pending {
                        reply(TCP_S_STATE, 0, 0);
                    } else {
                        st8.tcp.state = TCP_CLOSING;
                        tcp_send_new(st8, tcp::FIN | tcp::ACK, &[]);
                        reply(ARP_S_OK, 0, 0);
                    }
                }
                TCP_OP_RELEASE => {
                    if !st8.tcp.live || st8.tcp.handle != arg {
                        reply(TCP_S_BAD_HANDLE, 0, 0);
                    } else if st8.tcp.state != TCP_CLOSED && st8.tcp.state != TCP_FAILED {
                        reply(TCP_S_STATE, 0, 0);
                    } else {
                        st8.tcp = NO_TCP;
                        reply(ARP_S_OK, 0, 0);
                    }
                }
                DNS_OP_LOOKUP => {
                    let n = arg as usize;
                    if n == 0 || n > dns::NAME_MAX {
                        reply(DNS_S_BAD_NAME, 0, 0);
                        continue;
                    }
                    match dns_lookup(st8, &inbox[..n]) {
                        Ok(ip) => reply(
                            ARP_S_OK,
                            (ip[0] as u64)
                                | ((ip[1] as u64) << 8)
                                | ((ip[2] as u64) << 16)
                                | ((ip[3] as u64) << 24),
                            0,
                        ),
                        Err(status) => reply(status, 0, 0),
                    }
                }
                UDP_OP_RECV_CHUNK => {
                    let Some(i) = binding_of(st8, arg) else {
                        st8.bad_handles += 1;
                        reply(UDP_S_BAD_HANDLE, 0, 0);
                        continue;
                    };
                    // Only the holder of the SAME handle can read the
                    // continuation. No implicit access by port or index.
                    let off = u16::from_be_bytes([inbox[0], inbox[1]]) as usize;
                    let b = &st8.bindings[i];
                    if !b.staged || off < UDP_INLINE || off >= b.len {
                        reply(ARP_S_BAD_OP, 0, 0);
                        continue;
                    }
                    let n = core::cmp::min(MSG_BYTES, b.len - off);
                    let rr = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        ARP_S_OK,
                        n as u64,
                        CAP_NONE,
                        b.data[off..].as_ptr() as u64,
                    );
                    if rr < 0 {
                        fail(EXIT_REPLY, "the UDP chunk reply was refused");
                    }
                    if off + n == b.len {
                        st8.bindings[i].staged = false;
                    }
                }
                UDP_OP_CLOSE => match binding_of(st8, arg) {
                    Some(i) => {
                        st8.bindings[i] = NO_BINDING;
                        reply(ARP_S_OK, 0, 0);
                    }
                    None => {
                        st8.bad_handles += 1;
                        reply(UDP_S_BAD_HANDLE, 0, 0);
                    }
                },
                ICMP_OP_RXSTATS => reply(
                    ARP_S_OK,
                    (st8.rx_frames & 0xFFFF)
                        | ((st8.rx_arp & 0xFFFF) << 16)
                        | ((st8.rx_ipv4 & 0xFFFF) << 32)
                        | ((st8.rx_dropped & 0xFFFF) << 48),
                    0,
                ),
                ARP_OP_STATS => reply(
                    ARP_S_OK,
                    (st8.hits & 0xFFFF_FFFF) | (st8.wire_requests << 32),
                    0,
                ),
                ARP_OP_SHUTDOWN => {
                    log_line(|o| {
                        o.str("netstackd: shutdown — ");
                        o.u64(st8.wire_requests);
                        o.str(" ARP request(s) on the wire, ");
                        o.u64(st8.replies_seen);
                        o.str(" reply/replies, ");
                        o.u64(st8.hits);
                        o.str(" cache hit(s), ");
                        o.u64(st8.timeouts);
                        o.str(" timeout(s), ");
                        o.u64(st8.reattaches);
                        o.str(" re-attach(es); demux saw ");
                        o.u64(st8.rx_frames);
                        o.str(" frame(s): ");
                        o.u64(st8.rx_arp);
                        o.str(" ARP, ");
                        o.u64(st8.rx_ipv4);
                        o.str(" IPv4, ");
                        o.u64(st8.rx_dropped);
                        o.str(" dropped, ");
                        o.u64(st8.rx_bad_checksum);
                        o.str(" bad checksum; ");
                        o.u64(st8.rx_chunks);
                        o.str(" continuation chunk(s) read, ");
                        o.u64(st8.rx_oversize);
                        o.str(" oversize, ");
                        o.u64(st8.rx_corrupt_payload);
                        o.str(" corrupt payload");
                    });
                    // A SECOND line: one Out::push is capped at
                    // WRITE_MAX (256) and silently drops the rest, so
                    // a long enough accounting line loses its tail
                    // mid-word — which is exactly how this one failed
                    // its own assertion.
                    log_line(|o| {
                        o.str("netstackd: UDP — ");
                        o.u64(st8.rx_udp);
                        o.str(" delivered, ");
                        o.u64(st8.rx_udp_unbound);
                        o.str(" unbound, ");
                        o.u64(st8.bad_handles);
                        o.str(" bad handle(s) refused");
                    });
                    reply(
                        ARP_S_OK,
                        (st8.hits & 0xFFFF_FFFF) | (st8.wire_requests << 32),
                        0,
                    );
                    syscall1(SYS_THREAD_EXIT, EXIT_OK);
                }
                _ => reply(ARP_S_BAD_OP, 0, 0),
            }
        }
    }
}

/// The driver died under us: wait for it to come back, then
/// RE-ESTABLISH — which is not the same as retrying.
///
/// A restarted netd is a new process that has re-run its own virtio
/// handshake: fresh queues, freshly posted receive buffers, a fresh
/// MSI-X relay. Nothing it held for us survived, so the stack must
/// re-acquire its device facts rather than assume continuity. Here
/// that means one thing — ask the new instance for the MAC — and the
/// success of that call is also how the stack learns the driver is
/// alive again.
///
/// What this deliberately does NOT do is re-send whatever failed.
/// `STATUS_SERVICE_GONE` means the outcome is UNKNOWN (ADR-0028), and
/// for a datagram the frame may already be on the wire. Only the
/// CALLER decides whether its operation may be repeated; `resolve`
/// may, because an ARP request is a broadcast query.
///
/// The wait is a real timer on a real notification. Spinning on
/// `SYS_CLOCK_NOW` would be the polling this phase forbids, and the
/// stack cannot be woken by the supervisor — it has no way to know a
/// spawn happened except by asking.
///
/// # Safety
/// As `_start`.
unsafe fn reattach(st8: &mut Stack) -> bool {
    // SAFETY: function contract.
    unsafe {
        log("netstackd: the driver is GONE — waiting for the supervisor, then re-establishing");
        for attempt in 1..=REATTACH_TRIES {
            let id = syscall3(
                SYS_TIMER_ARM,
                SLOT_NOTIF,
                BADGE_BACKOFF,
                REATTACH_BACKOFF_US,
            );
            if id >= 0 {
                syscall1(SYS_WAIT, SLOT_NOTIF);
            }
            let (status, packed) = netd_call(NET_OP_MAC, 0, CAP_NONE, 0);
            if status == NET_S_OK && packed != 0 {
                for (i, m) in st8.mac.iter_mut().enumerate() {
                    *m = ((packed >> (8 * i)) & 0xFF) as u8;
                }
                st8.reattaches += 1;
                log_line(|o| {
                    o.str("netstackd: RE-ATTACHED to the restarted driver on attempt ");
                    o.u64(attempt as u64);
                    o.str(" — device facts re-acquired, nothing assumed to have survived");
                });
                return true;
            }
        }
        log("netstackd: the driver never came back — reporting the link down rather than guessing");
        false
    }
}

/// Draw the UDP handle pool from rngd (M7.4).
///
/// A handle is authority by possession, so it has to be unguessable —
/// an index would be authority by arithmetic. The draw happens once,
/// at startup, into the transmit frame before any packet has used it;
/// the pool is then just numbers in this service's memory and rngd is
/// never needed again.
///
/// If entropy is unavailable the stack does NOT fall back to
/// something predictable and call it a handle. It records that it has
/// none and refuses to bind, which is honest and safe; a guessable
/// token would be worse than an obvious refusal.
///
/// # Safety
/// As `_start`.
unsafe fn draw_handles(st8: &mut Stack) {
    // SAFETY: function contract.
    unsafe {
        let want = (UDP_BINDINGS * 8) as u64;
        let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
        if syscall3(SYS_CAP_COPY, SLOT_TX_MASTER, SLOT_TX_LENT, RIGHTS_ALL) < 0 {
            log("netstackd: no entropy — the handle cap copy was refused; UDP will refuse to bind");
            return;
        }
        let mut reply = [0u64; 3];
        let r = syscall6(
            SYS_IPC_CALL,
            SLOT_RNG,
            want,
            RNG_OP_GET,
            SLOT_TX_LENT,
            reply.as_mut_ptr() as u64,
            0,
        );
        if r < 0 || reply[0] != RNG_S_OK || reply[1] < want {
            log(
                "netstackd: no entropy — UDP will refuse to bind rather than issue a guessable handle",
            );
            return;
        }
        for i in 0..UDP_BINDINGS {
            let mut v = 0u64;
            for b in 0..8 {
                v |= (r8(st8.tx + (i * 8 + b) as u64) as u64) << (8 * b);
            }
            // Zero or duplicate random values fail CLOSED. Even an
            // astronomically unlikely draw must not turn into a
            // predictable token or share authority with another port.
            if v == 0 || st8.handle_pool[..i].contains(&v) {
                log("netstackd: entropy draw unusable — UDP refusing to bind");
                return;
            }
            st8.handle_pool[i] = v;
        }
        st8.have_entropy = true;
        log_line(|o| {
            o.str("netstackd: drew ");
            o.u64(UDP_BINDINGS as u64);
            o.str(" unguessable UDP handles from rngd — possession of one IS the authority to use its port");
        });
    }
}

/// Obtain a fresh bearer token when reusing a binding slot. The old
/// token is dead at CLOSE and cannot revive even if the same port is
/// later bound again. Refusal on entropy loss is part of the contract.
///
/// # Safety
/// As `_start`.
unsafe fn refresh_handle(st8: &mut Stack, i: usize) -> bool {
    // SAFETY: the only thread owns these caps and the mapped tx frame.
    unsafe {
        let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
        if syscall3(SYS_CAP_COPY, SLOT_TX_MASTER, SLOT_TX_LENT, RIGHTS_ALL) < 0 {
            return false;
        }
        let mut reply = [0u64; 3];
        let r = syscall6(
            SYS_IPC_CALL,
            SLOT_RNG,
            8,
            RNG_OP_GET,
            SLOT_TX_LENT,
            reply.as_mut_ptr() as u64,
            0,
        );
        if r < 0 || reply[0] != RNG_S_OK || reply[1] < 8 {
            return false;
        }
        let mut v = 0u64;
        for b in 0..8 {
            v |= (r8(st8.tx + b as u64) as u64) << (8 * b);
        }
        if v == 0 || st8.handle_pool.contains(&v) {
            return false;
        }
        st8.handle_pool[i] = v;
        true
    }
}

/// Find a binding by handle. The ONLY way to reach a binding.
fn binding_of(st8: &mut Stack, handle: u64) -> Option<usize> {
    if handle == 0 {
        return None;
    }
    (0..UDP_BINDINGS).find(|&i| st8.bindings[i].live && st8.bindings[i].handle == handle)
}

/// A UDP datagram arrived for us: hand it to the binding that owns the
/// destination port, or count it and drop it.
///
/// # Safety
/// `udp` points at `len` readable bytes.
unsafe fn udp_in(st8: &mut Stack, src: [u8; 4], udp: *const u8, len: usize) {
    // SAFETY: caller contract.
    unsafe {
        if len < UDP_HDR {
            st8.rx_dropped += 1;
            return;
        }
        let at = |i: usize| *udp.add(i);
        let src_port = ((at(0) as u16) << 8) | at(1) as u16;
        let dst_port = ((at(2) as u16) << 8) | at(3) as u16;
        let ulen = (((at(4) as u16) << 8) | at(5) as u16) as usize;
        // The UDP length covers header + payload and must fit what the
        // IP layer declared. (The checksum is optional in IPv4 UDP and
        // slirp may send zero; a zero checksum is accepted, a nonzero
        // one is NOT yet verified — stated in ADR-0033 rather than
        // quietly skipped.)
        if ulen < UDP_HDR || ulen > len {
            st8.rx_dropped += 1;
            return;
        }
        let wire_ck = ((at(6) as u16) << 8) | at(7) as u16;
        if wire_ck != 0 && udp_checksum(src, SLIRP_GUEST_IP, udp, ulen) != 0 {
            st8.rx_bad_checksum += 1;
            st8.rx_dropped += 1;
            return;
        }
        let Some(i) =
            (0..UDP_BINDINGS).find(|&i| st8.bindings[i].live && st8.bindings[i].port == dst_port)
        else {
            st8.rx_udp_unbound += 1;
            st8.rx_dropped += 1;
            return;
        };
        let payload = ulen - UDP_HDR;
        if payload > UDP_MAX || st8.bindings[i].staged {
            st8.rx_dropped += 1;
            return;
        }
        let b = &mut st8.bindings[i];
        for k in 0..payload {
            b.data[k] = at(UDP_HDR + k);
        }
        b.len = payload;
        b.src_ip = src;
        b.src_port = src_port;
        b.have = true;
        st8.rx_udp += 1;
    }
}

/// Lay out a UDP datagram: Ethernet, IPv4, UDP, payload.
///
/// # Safety
/// `st8.tx` is this image's own mapped frame; `payload` is readable.
unsafe fn build_udp(
    st8: &Stack,
    mac: [u8; 6],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: *const u8,
    plen: usize,
) -> u64 {
    // SAFETY: method contract; everything written is inside one page.
    unsafe {
        let base = st8.tx;
        let put = |off: usize, b: u8| *((base + off as u64) as *mut u8) = b;
        for (i, (dst, src)) in mac.iter().zip(st8.mac.iter()).enumerate() {
            put(i, *dst);
            put(6 + i, *src);
        }
        put(12, (ETHERTYPE_IPV4 >> 8) as u8);
        put(13, (ETHERTYPE_IPV4 & 0xFF) as u8);
        let total = (IP_HDR + UDP_HDR + plen) as u16;
        put(ETH_HDR, 0x45);
        put(ETH_HDR + 1, 0);
        put(ETH_HDR + 2, (total >> 8) as u8);
        put(ETH_HDR + 3, (total & 0xFF) as u8);
        put(ETH_HDR + 4, 0);
        put(ETH_HDR + 5, 0);
        put(ETH_HDR + 6, 0x40);
        put(ETH_HDR + 7, 0);
        put(ETH_HDR + 8, IP_TTL);
        put(ETH_HDR + 9, IP_PROTO_UDP);
        put(ETH_HDR + 10, 0);
        put(ETH_HDR + 11, 0);
        for i in 0..4 {
            put(ETH_HDR + 12 + i, SLIRP_GUEST_IP[i]);
            put(ETH_HDR + 16 + i, dst_ip[i]);
        }
        let ck = checksum((base + ETH_HDR as u64) as *const u8, IP_HDR);
        put(ETH_HDR + 10, (ck >> 8) as u8);
        put(ETH_HDR + 11, (ck & 0xFF) as u8);
        let u = ETH_HDR + IP_HDR;
        put(u, (src_port >> 8) as u8);
        put(u + 1, (src_port & 0xFF) as u8);
        put(u + 2, (dst_port >> 8) as u8);
        put(u + 3, (dst_port & 0xFF) as u8);
        let ulen = (UDP_HDR + plen) as u16;
        put(u + 4, (ulen >> 8) as u8);
        put(u + 5, (ulen & 0xFF) as u8);
        put(u + 6, 0);
        put(u + 7, 0);
        for i in 0..plen {
            put(u + UDP_HDR + i, *payload.add(i));
        }
        let ck = udp_checksum(
            SLIRP_GUEST_IP,
            dst_ip,
            (base + u as u64) as *const u8,
            ulen as usize,
        );
        // Zero ON THE WIRE means "checksum absent" in IPv4 UDP.
        let ck = if ck == 0 { 0xFFFF } else { ck };
        put(u + 6, (ck >> 8) as u8);
        put(u + 7, (ck & 0xFF) as u8);
        (ETH_HDR + IP_HDR + UDP_HDR + plen) as u64
    }
}

/// Draw a fresh DNS transaction id from rngd. A hardcoded id (like the
/// UDP transport test's) is not adequate for a resolver: a spoofed reply
/// must have to guess both the source tuple AND this id.
///
/// # Safety
/// As `_start`; tx is a mapped page and SLOT_TX_MASTER is lent.
unsafe fn dns_id(st8: &Stack) -> Option<u16> {
    // SAFETY: caller contract.
    unsafe {
        let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
        if syscall3(SYS_CAP_COPY, SLOT_TX_MASTER, SLOT_TX_LENT, RIGHTS_ALL) < 0 {
            return None;
        }
        let mut reply = [0u64; 3];
        let r = syscall6(
            SYS_IPC_CALL,
            SLOT_RNG,
            2,
            RNG_OP_GET,
            SLOT_TX_LENT,
            reply.as_mut_ptr() as u64,
            0,
        );
        if r < 0 || reply[0] != RNG_S_OK || reply[1] < 2 {
            return None;
        }
        // rngd filled the first two bytes of the SAME mapped tx page.
        Some(u16::from_be_bytes([r8(st8.tx), r8(st8.tx + 1)]))
    }
}

/// DNS lookup is one bounded transaction. No automatic resend: a SEND
/// failure means the frame may already have reached the wire (ADR-0030).
/// Other frames still pass through the demultiplexer while we wait.
///
/// # Safety
/// As `_start`.
unsafe fn dns_lookup(st8: &mut Stack, name: &[u8]) -> Result<[u8; 4], u64> {
    // SAFETY: function contract; query and UDP stage are bounded.
    unsafe {
        let mut q = [0u8; dns::QUERY_MAX];
        // Validate BEFORE touching network or entropy.
        if dns::query(name, 1, &mut q).is_none() {
            return Err(DNS_S_BAD_NAME);
        }
        let Some(slot) = (0..UDP_BINDINGS).find(|&i| !st8.bindings[i].live) else {
            return Err(DNS_S_BUSY);
        };
        let Some(mac) = resolve(st8, [10, 0, 2, 3]) else {
            return Err(ARP_S_UNREACHABLE);
        };
        let Some(id) = dns_id(st8) else {
            return Err(ARP_S_LINK_DOWN);
        };
        let qlen = dns::query(name, id, &mut q).ok_or(DNS_S_BAD_NAME)?;
        // Internal binding: handle 0 cannot be presented to binding_of.
        st8.bindings[slot] = Binding {
            live: true,
            port: DNS_PORT,
            ..NO_BINDING
        };
        let result = (|| {
            let flen = build_udp(st8, mac, [10, 0, 2, 3], DNS_PORT, 53, q.as_ptr(), qlen);
            if transmit(st8, flen) != NET_S_OK {
                return Err(ARP_S_LINK_DOWN);
            }
            let deadline = (syscall0(SYS_CLOCK_NOW).max(0) as u64).saturating_add(DNS_TIMEOUT_US);
            while !st8.bindings[slot].have {
                let remaining = deadline.saturating_sub(syscall0(SYS_CLOCK_NOW).max(0) as u64);
                if remaining == 0 || !pump(st8, remaining) {
                    return Err(DNS_S_NO_ANSWER);
                }
            }
            let b = st8.bindings[slot];
            if b.src_ip != [10, 0, 2, 3] || b.src_port != 53 {
                return Err(DNS_S_BAD_REPLY);
            }
            dns::answer(&b.data[..b.len], id, &q[..qlen]).ok_or(DNS_S_BAD_REPLY)
        })();
        st8.bindings[slot] = NO_BINDING;
        result
    }
}

/// Draw TCP authority, initial sequence and source port from rngd. No
/// entropy means no active connection: a predictable bearer is not one.
///
/// # Safety
/// As `_start`; rngd DMAs into our mapped transmit page.
unsafe fn tcp_entropy(st8: &Stack) -> Option<(u64, u32, u16)> {
    // SAFETY: the only thread owns these caps and the mapped frame.
    unsafe {
        let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
        if syscall3(SYS_CAP_COPY, SLOT_TX_MASTER, SLOT_TX_LENT, RIGHTS_ALL) < 0 {
            return None;
        }
        let mut reply = [0u64; 3];
        let r = syscall6(
            SYS_IPC_CALL,
            SLOT_RNG,
            16,
            RNG_OP_GET,
            SLOT_TX_LENT,
            reply.as_mut_ptr() as u64,
            0,
        );
        if r < 0 || reply[0] != RNG_S_OK || reply[1] < 16 {
            return None;
        }
        let mut b = [0u8; 16];
        for (i, v) in b.iter_mut().enumerate() {
            *v = r8(st8.tx + i as u64);
        }
        let handle = u64::from_le_bytes(b[..8].try_into().ok()?);
        if handle == 0 {
            return None;
        }
        let seq = u32::from_le_bytes(b[8..12].try_into().ok()?);
        let port = 49152 + (u16::from_le_bytes(b[12..14].try_into().ok()?) % 16384);
        Some((handle, seq, port))
    }
}

/// # Safety
/// As `_start`.
unsafe fn tcp_open(st8: &mut Stack, ip: [u8; 4], port: u16) -> Result<u64, u64> {
    // SAFETY: caller contract; no externally reachable state before validation.
    unsafe {
        if port == 0 {
            return Err(ARP_S_BAD_OP);
        }
        if st8.tcp.live {
            return Err(TCP_S_BUSY);
        }
        let mac = resolve(st8, ip).ok_or(ARP_S_UNREACHABLE)?;
        let (handle, seq, src_port) = tcp_entropy(st8).ok_or(ARP_S_LINK_DOWN)?;
        st8.tcp = TcpConn {
            live: true,
            handle,
            ip,
            mac,
            src_port,
            dst_port: port,
            state: TCP_CONNECTING,
            snd_nxt: seq,
            ..NO_TCP
        };
        tcp_send_new(st8, tcp::SYN, &[]);
        Ok(handle)
    }
}

/// Send one TCP segment. Caller owns the one mapped frame, and the
/// driver accepts a lent COPY of it; it never gets device authority.
/// # Safety
/// As `_start`; data is a bounded slice of the caller's memory.
unsafe fn tcp_send_raw(st8: &mut Stack, flags: u8, seq: u32, data: &[u8]) -> bool {
    // SAFETY: the mapped page covers the entire Ethernet/IP/TCP segment.
    unsafe {
        let base = st8.tx;
        let put = |i: usize, byte: u8| *((base + i as u64) as *mut u8) = byte;
        for i in 0..6 {
            put(i, st8.tcp.mac[i]);
            put(6 + i, st8.mac[i]);
        }
        put(12, (ETHERTYPE_IPV4 >> 8) as u8);
        put(13, ETHERTYPE_IPV4 as u8);
        let start = ETH_HDR + IP_HDR;
        let segment = core::slice::from_raw_parts_mut(
            (base + start as u64) as *mut u8,
            tcp::HDR + 4 + MSG_BYTES,
        );
        let ack = if flags & tcp::ACK != 0 {
            st8.tcp.rcv_nxt
        } else {
            0
        };
        let len = tcp::write(
            segment,
            SLIRP_GUEST_IP,
            st8.tcp.ip,
            st8.tcp.src_port,
            st8.tcp.dst_port,
            seq,
            ack,
            flags,
            (tcp::MSS - st8.tcp.rx_len) as u16,
            data,
        );
        let total = (IP_HDR + len) as u16;
        put(ETH_HDR, 0x45);
        put(ETH_HDR + 1, 0);
        put(ETH_HDR + 2, (total >> 8) as u8);
        put(ETH_HDR + 3, total as u8);
        put(ETH_HDR + 4, 0);
        put(ETH_HDR + 5, 0);
        put(ETH_HDR + 6, 0x40); // DF; v1 will not reassemble IP fragments
        put(ETH_HDR + 7, 0);
        put(ETH_HDR + 8, IP_TTL);
        put(ETH_HDR + 9, IP_PROTO_TCP);
        put(ETH_HDR + 10, 0);
        put(ETH_HDR + 11, 0);
        for (i, b) in SLIRP_GUEST_IP.iter().enumerate() {
            put(ETH_HDR + 12 + i, *b);
            put(ETH_HDR + 16 + i, st8.tcp.ip[i]);
        }
        let ck = checksum((base + ETH_HDR as u64) as *const u8, IP_HDR);
        put(ETH_HDR + 10, (ck >> 8) as u8);
        put(ETH_HDR + 11, ck as u8);
        transmit(st8, (ETH_HDR + IP_HDR + len) as u64) == NET_S_OK
    }
}

/// Create an outstanding segment before TX. If netd dies during SEND,
/// outcome is unknown: retain the sequence and let the RTO retransmit
/// after re-attachment. Never create a second sequence by blind retry.
/// # Safety
/// As `_start`.
unsafe fn tcp_send_new(st8: &mut Stack, flags: u8, data: &[u8]) {
    // SAFETY: single outstanding segment and one TX frame.
    unsafe {
        let seq = st8.tcp.snd_nxt;
        st8.tcp.pending = true;
        st8.tcp.pending_seq = seq;
        st8.tcp.pending_flags = flags;
        st8.tcp.pending_len = data.len();
        st8.tcp.pending_data[..data.len()].copy_from_slice(data);
        st8.tcp.snd_nxt = seq
            .wrapping_add(data.len() as u32)
            .wrapping_add(u32::from(flags & tcp::SYN != 0))
            .wrapping_add(u32::from(flags & tcp::FIN != 0));
        st8.tcp.sent_us = syscall0(SYS_CLOCK_NOW).max(0) as u64;
        st8.tcp.retries = 0;
        let _ = tcp_send_raw(st8, flags, seq, data);
    }
}

/// One blocking device wait at most. POLL returns to the client after
/// a state change or a bounded deadline, not a service-side busy loop.
/// # Safety
/// As `_start`.
unsafe fn tcp_poll(st8: &mut Stack, timeout_us: u64) {
    // SAFETY: the only thread pumps the shared receive demultiplexer.
    unsafe {
        if st8.tcp.state == TCP_CLOSED || st8.tcp.state == TCP_FAILED || st8.tcp.rx_len > 0 {
            return;
        }
        let now = syscall0(SYS_CLOCK_NOW).max(0) as u64;
        let wait = if st8.tcp.pending {
            core::cmp::min(
                timeout_us,
                st8.tcp
                    .sent_us
                    .saturating_add(tcp::RTO_US)
                    .saturating_sub(now),
            )
        } else {
            timeout_us
        };
        // zero is netd's nonblocking poll, never an unbounded wait.
        let _ = pump(st8, wait);
        let now = syscall0(SYS_CLOCK_NOW).max(0) as u64;
        if st8.tcp.pending {
            match tcp::retry_action(st8.tcp.sent_us, now, st8.tcp.retries) {
                tcp::Retry::Wait => {}
                tcp::Retry::Fail => {
                    st8.tcp.state = TCP_FAILED;
                    st8.tcp.pending = false;
                }
                tcp::Retry::Resend => {
                    st8.tcp.retries += 1;
                    st8.tcp.retransmits += 1;
                    let p = st8.tcp;
                    st8.tcp.sent_us = now;
                    let _ = tcp_send_raw(
                        st8,
                        p.pending_flags,
                        p.pending_seq,
                        &p.pending_data[..p.pending_len],
                    );
                }
            }
        }
    }
}

/// The TCP arm of the IPv4 demultiplexer. Only an exact source tuple,
/// valid pseudo-header checksum, and expected sequence can advance state.
/// # Safety
/// `wire` is the checked IP payload from a bounded frame.
unsafe fn tcp_in(st8: &mut Stack, src: [u8; 4], wire: &[u8]) {
    // SAFETY: parser bounds each header field before it is used.
    unsafe {
        let seg = match tcp::parse(src, SLIRP_GUEST_IP, wire) {
            Ok(seg) => seg,
            Err(tcp::ParseError::Checksum) => {
                st8.rx_bad_checksum += 1;
                st8.rx_dropped += 1;
                return;
            }
            Err(_) => {
                st8.rx_dropped += 1;
                return;
            }
        };
        if !st8.tcp.live
            || src != st8.tcp.ip
            || seg.src_port != st8.tcp.dst_port
            || seg.dst_port != st8.tcp.src_port
        {
            st8.rx_dropped += 1;
            return;
        }
        if seg.flags & tcp::RST != 0 {
            // An RST in-window only; an unrelated injected RST does not
            // cancel a connection by guessing its four-tuple.
            if seg.seq == st8.tcp.rcv_nxt
                || (st8.tcp.state == TCP_CONNECTING
                    && seg.flags & tcp::ACK != 0
                    && seg.ack == st8.tcp.snd_nxt)
            {
                st8.tcp.state = TCP_FAILED;
                st8.tcp.pending = false;
            }
            return;
        }
        if st8.tcp.state == TCP_CONNECTING {
            if seg.flags & (tcp::SYN | tcp::ACK) != (tcp::SYN | tcp::ACK)
                || seg.ack != st8.tcp.snd_nxt
                || !seg.data.is_empty()
            {
                st8.rx_dropped += 1;
                return;
            }
            st8.tcp.rcv_nxt = seg.seq.wrapping_add(1);
            st8.tcp.peer_window = seg.window;
            st8.tcp.pending = false;
            st8.tcp.state = TCP_ESTABLISHED;
            let _ = tcp_send_raw(st8, tcp::ACK, st8.tcp.snd_nxt, &[]);
            return;
        }
        if st8.tcp.state != TCP_ESTABLISHED && st8.tcp.state != TCP_CLOSING {
            return;
        }
        if seg.flags & tcp::ACK == 0 {
            st8.rx_dropped += 1;
            return;
        }
        st8.tcp.peer_window = seg.window;
        if st8.tcp.pending && seg.ack == st8.tcp.snd_nxt {
            st8.tcp.pending = false;
        } else if seg.ack != st8.tcp.snd_nxt && seg.ack != st8.tcp.pending_seq {
            st8.rx_dropped += 1;
            return;
        }
        if seg.seq != st8.tcp.rcv_nxt || seg.data.len() > tcp::MSS - st8.tcp.rx_len {
            if seg.seq != st8.tcp.rcv_nxt || !seg.data.is_empty() {
                let _ = tcp_send_raw(st8, tcp::ACK, st8.tcp.snd_nxt, &[]);
            }
            // ACK processing above is independent of inbound sequence.
            // A peer may retransmit its already-ACKed FIN while ACKing
            // OUR FIN. The duplicate consumes no sequence, but its ACK
            // still completes the close; returning first hangs forever.
            if st8.tcp.state == TCP_CLOSING && st8.tcp.peer_fin && !st8.tcp.pending {
                st8.tcp.state = TCP_CLOSED;
            }
            return;
        }
        if !seg.data.is_empty() {
            let start = st8.tcp.rx_len;
            st8.tcp.rx[start..start + seg.data.len()].copy_from_slice(seg.data);
            st8.tcp.rx_len += seg.data.len();
            st8.tcp.rcv_nxt = st8.tcp.rcv_nxt.wrapping_add(seg.data.len() as u32);
        }
        if seg.flags & tcp::FIN != 0 && !st8.tcp.peer_fin {
            st8.tcp.peer_fin = true;
            st8.tcp.rcv_nxt = st8.tcp.rcv_nxt.wrapping_add(1);
        }
        if !seg.data.is_empty() || seg.flags & tcp::FIN != 0 {
            let _ = tcp_send_raw(st8, tcp::ACK, st8.tcp.snd_nxt, &[]);
        }
        if st8.tcp.state == TCP_CLOSING && st8.tcp.peer_fin && !st8.tcp.pending {
            st8.tcp.state = TCP_CLOSED;
        }
    }
}

/// Read one whole frame from the driver into `buf`, in chunks.
///
/// netd hands back a frame's FULL length with its first chunk and the
/// rest by offset (M7.3, ADR-0032). Before that a frame larger than
/// one IPC message was dropped by the driver, which quietly made this
/// a stack for small packets only.
///
/// Returns the frame length, or 0 for "nothing usable arrived".
///
/// # Safety
/// As `_start`.
unsafe fn recv_frame(st8: &mut Stack, buf: &mut [u8; FRAME_MAX], timeout_us: u64) -> usize {
    // SAFETY: function contract.
    unsafe {
        let mut chunk = [0u64; MSG_BYTES / 8];
        let (status, flen) =
            netd_call(NET_OP_RECV, timeout_us, CAP_NONE, chunk.as_mut_ptr() as u64);
        if status == NET_S_TIMEOUT {
            st8.timeouts += 1;
            return 0;
        }
        if status == ARP_S_LINK_DOWN {
            reattach(st8);
            return 0;
        }
        if status != NET_S_OK {
            return 0;
        }
        let total = flen as usize;
        let keep = core::cmp::min(total, FRAME_MAX);
        if total > FRAME_MAX {
            st8.rx_oversize += 1;
        }
        let first = core::cmp::min(total, MSG_BYTES);
        core::ptr::copy_nonoverlapping(
            chunk.as_ptr() as *const u8,
            buf.as_mut_ptr(),
            core::cmp::min(first, keep),
        );
        let mut off = first;
        // Every remaining chunk is read even when the frame is too
        // long to keep: the driver only returns its receive buffer to
        // the ring when the LAST chunk is taken, so abandoning one
        // here would leak a buffer per oversize frame.
        while off < total {
            let (st, _) = netd_call(
                NET_OP_RECV_CHUNK,
                off as u64,
                CAP_NONE,
                chunk.as_mut_ptr() as u64,
            );
            if st != NET_S_OK {
                return 0;
            }
            let n = core::cmp::min(total - off, MSG_BYTES);
            if off < keep {
                core::ptr::copy_nonoverlapping(
                    chunk.as_ptr() as *const u8,
                    buf.as_mut_ptr().add(off),
                    core::cmp::min(n, keep - off),
                );
            }
            off += n;
            st8.rx_chunks += 1;
        }
        if total > FRAME_MAX { 0 } else { total }
    }
}

/// The receive demultiplexer (M7.2): pull ONE frame from the driver
/// and give it to whoever it belongs to.
///
/// ADR-0030 called the receive path "the seam everything else hangs
/// from", and this is it. In 7.1 there was exactly one consumer, so
/// `resolve` could read frames itself and drop anything that was not
/// an ARP reply. The moment a second protocol exists that is wrong:
/// an ICMP reply arriving while the stack happens to be resolving
/// would be thrown away, and the ping that was waiting for it would
/// time out for no reason. So recognising a frame is now one job, in
/// one place, and the operations wait on STATE that this updates.
///
/// Returns false when the driver reported a timeout (nothing arrived)
/// or is gone — the caller decides what that means for its own
/// operation.
///
/// # Safety
/// As `_start`.
unsafe fn pump(st8: &mut Stack, timeout_us: u64) -> bool {
    // SAFETY: function contract.
    unsafe {
        let mut frame = [0u8; FRAME_MAX];
        let len = recv_frame(st8, &mut frame, timeout_us);
        if len == 0 {
            return false;
        }
        st8.rx_frames += 1;
        demux(st8, frame.as_ptr(), len);
        true
    }
}

/// Dispatch one received frame by ethertype.
///
/// # Safety
/// `bytes` points at `len` readable bytes of this image's memory.
unsafe fn demux(st8: &mut Stack, bytes: *const u8, len: usize) {
    // SAFETY: caller contract; every read below is bounds-checked
    // against `len` first.
    unsafe {
        if len < ETH_HDR {
            st8.rx_dropped += 1;
            return;
        }
        let at = |i: usize| *bytes.add(i);
        let ethertype = ((at(12) as u16) << 8) | at(13) as u16;
        match ethertype {
            ETHERTYPE_ARP => {
                st8.rx_arp += 1;
                arp_in(st8, bytes, len);
            }
            ETHERTYPE_IPV4 => {
                st8.rx_ipv4 += 1;
                ipv4_in(st8, bytes, len);
            }
            _ => {
                // Not ours. Counted, so "we dropped it" is a fact the
                // stack can be asked about rather than a silence.
                st8.rx_dropped += 1;
            }
        }
    }
}

/// An ARP frame arrived: is it the reply we are waiting for?
///
/// # Safety
/// As `demux`.
unsafe fn arp_in(st8: &mut Stack, bytes: *const u8, len: usize) {
    // SAFETY: caller contract.
    unsafe {
        let Some(want) = st8.want_arp else {
            st8.rx_dropped += 1;
            return;
        };
        match parse_reply(bytes, len, want) {
            Some(mac) => {
                st8.got_arp = Some(mac);
                st8.replies_seen += 1;
            }
            None => st8.rx_dropped += 1,
        }
    }
}

/// The internet checksum (RFC 1071): ones' complement of the ones'
/// complement sum of 16-bit words.
///
/// # Safety
/// `bytes` points at `len` readable bytes.
unsafe fn checksum(bytes: *const u8, len: usize) -> u16 {
    // SAFETY: caller contract.
    unsafe {
        let mut sum: u32 = 0;
        let mut i = 0;
        while i + 1 < len {
            sum += ((*bytes.add(i) as u32) << 8) | *bytes.add(i + 1) as u32;
            i += 2;
        }
        if i < len {
            sum += (*bytes.add(i) as u32) << 8;
        }
        while sum >> 16 != 0 {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        !(sum as u16)
    }
}

/// UDP checksum includes the IP pseudo-header; the IPv4 header checksum
/// alone does NOT protect the addresses, protocol or UDP contents.
/// A zero result on reception (when the wire field was nonzero) verifies.
///
/// # Safety
/// `udp` points to `len` readable bytes.
unsafe fn udp_checksum(src: [u8; 4], dst: [u8; 4], udp: *const u8, len: usize) -> u16 {
    let mut pseudo = [0u8; 12];
    pseudo[..4].copy_from_slice(&src);
    pseudo[4..8].copy_from_slice(&dst);
    pseudo[9] = IP_PROTO_UDP;
    pseudo[10..12].copy_from_slice(&(len as u16).to_be_bytes());
    // SAFETY: caller's UDP buffer and our own pseudo-header are readable.
    let mut sum = unsafe { (!checksum(pseudo.as_ptr(), 12) as u32) + (!checksum(udp, len) as u32) };
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// An IPv4 frame arrived. Verify it before believing any of it.
///
/// # Safety
/// As `demux`.
unsafe fn ipv4_in(st8: &mut Stack, bytes: *const u8, len: usize) {
    // SAFETY: caller contract; offsets checked against `len`.
    unsafe {
        if len < ETH_HDR + IP_HDR {
            st8.rx_dropped += 1;
            return;
        }
        let ip = bytes.add(ETH_HDR);
        let at = |i: usize| *ip.add(i);
        let version = at(0) >> 4;
        let ihl = (at(0) & 0x0F) as usize * 4;

        // STRICT, and strict in the same terms the ADR uses. v1 said
        // "fixed 20-byte header, no options, no fragmentation"; the
        // first parser then accepted IHL > 5, ignored the fragment
        // bits, and measured the payload with the FRAME length
        // instead of the header's own. A parser that accepts more
        // than its specification is a specification nobody is
        // keeping (found in review of v0.14.0).
        if version != 4 || ihl != IP_HDR || len < ETH_HDR + IP_HDR {
            st8.rx_dropped += 1;
            return;
        }
        // The header checksum, CHECKED. A stack that skips this is
        // trusting the wire, and the wire is the one thing it must
        // not trust.
        if checksum(ip, ihl) != 0 {
            st8.rx_bad_checksum += 1;
            st8.rx_dropped += 1;
            return;
        }
        // Fragments: not supported, so not accepted. MF set or a
        // nonzero offset means this is a piece of something, and
        // treating a piece as a whole datagram is how a parser gets
        // told a lie it believes.
        let flags_frag = ((at(6) as u16) << 8) | at(7) as u16;
        if flags_frag & 0x2000 != 0 || flags_frag & 0x1FFF != 0 {
            st8.rx_fragments += 1;
            st8.rx_dropped += 1;
            return;
        }
        // The DECLARED length governs, and must fit in what arrived.
        // Ethernet pads short frames to 60 bytes, so the frame is
        // routinely longer than the datagram; measuring the payload
        // with the frame length feeds padding to the checksum and to
        // whatever parses next.
        let total = ((at(2) as usize) << 8) | at(3) as usize;
        if total < IP_HDR || ETH_HDR + total > len {
            st8.rx_dropped += 1;
            return;
        }
        // Addressed to us? (No forwarding: this is a host, not a
        // router, and says so.)
        for (i, b) in SLIRP_GUEST_IP.iter().enumerate() {
            if at(16 + i) != *b {
                st8.rx_dropped += 1;
                return;
            }
        }
        let proto = at(9);
        if proto != IP_PROTO_ICMP && proto != IP_PROTO_UDP && proto != IP_PROTO_TCP {
            st8.rx_dropped += 1;
            return;
        }
        let mut src = [0u8; 4];
        for (i, b) in src.iter_mut().enumerate() {
            *b = at(12 + i);
        }
        if proto == IP_PROTO_UDP {
            udp_in(st8, src, ip.add(ihl), total - ihl);
        } else if proto == IP_PROTO_TCP {
            let segment = core::slice::from_raw_parts(ip.add(ihl), total - ihl);
            tcp_in(st8, src, segment);
        } else {
            icmp_in(st8, src, ip.add(ihl), total - ihl);
        }
    }
}

/// An ICMP message arrived inside an IPv4 packet addressed to us,
/// from `src`, carrying `len` DECLARED bytes.
///
/// # Safety
/// `icmp` points at `len` readable bytes.
unsafe fn icmp_in(st8: &mut Stack, src: [u8; 4], icmp: *const u8, len: usize) {
    // SAFETY: caller contract.
    unsafe {
        if len < ICMP_HDR {
            st8.rx_dropped += 1;
            return;
        }
        if checksum(icmp, len) != 0 {
            st8.rx_bad_checksum += 1;
            st8.rx_dropped += 1;
            return;
        }
        let at = |i: usize| *icmp.add(i);
        // Echo REQUESTS are not answered: nothing here has asked to be
        // pingable, and shipping an untested reply path would be worse
        // than not having one (M7.2, ADR-0031). Code must be 0 — an
        // echo reply is only an echo reply at code 0.
        if at(0) != ICMP_ECHO_REPLY || at(1) != 0 {
            st8.rx_dropped += 1;
            return;
        }
        let id = ((at(4) as u16) << 8) | at(5) as u16;
        let seq = ((at(6) as u16) << 8) | at(7) as u16;
        // The SOURCE, the identifier and the sequence must all match
        // what went out. Without the source, a reply from a different
        // host carrying the same id/seq satisfies our ping — which on
        // a shared network is not hypothetical (v0.14.0 review).
        let from_expected = st8.want_icmp_ip == Some(src);
        match st8.want_icmp {
            Some((want_id, want_seq)) if from_expected && want_id == id && want_seq == seq => {
                // The PAYLOAD must come back byte-for-byte. This is
                // what actually proves the chunked receive path
                // (M7.3): a 200-byte echo crosses four IPC messages,
                // and a reassembly that dropped, duplicated or
                // reordered a chunk would still produce a frame with
                // the right identifier and sequence. The pattern is
                // position-dependent for the same reason.
                let mut intact = len >= ICMP_HDR + PING_PAYLOAD;
                if intact {
                    for i in 0..PING_PAYLOAD {
                        if at(ICMP_HDR + i) != b"arenaos!"[i % 8] ^ (i as u8) {
                            intact = false;
                            break;
                        }
                    }
                }
                if intact {
                    st8.got_icmp = true;
                } else {
                    st8.rx_corrupt_payload += 1;
                    st8.rx_dropped += 1;
                }
            }
            _ => {
                if !from_expected {
                    st8.rx_wrong_source += 1;
                }
                st8.rx_dropped += 1;
            }
        }
    }
}

/// Feed the receive parser frames the WIRE WILL NEVER SEND, and check
/// it refuses each one (M7.2, after the v0.14.0 review).
///
/// Every reject path in `ipv4_in`/`icmp_in` exists for a hostile or
/// broken peer, and slirp is neither: it will not send a fragment, an
/// options header, a truncated datagram, a bad checksum, or somebody
/// else's echo reply. So those paths would ship untested — which is
/// exactly how a parser ends up accepting more than its specification
/// while every test stays green.
///
/// Synthetic frames cost one function and run on every boot. Each
/// case asserts the counter that should move, so a regression shows
/// up as the parser silently ACCEPTING something, which is the
/// direction that matters.
///
/// Returns the number of cases that behaved correctly.
///
/// # Safety
/// As `_start`; operates only on a local buffer.
unsafe fn parser_selftest(st8: &mut Stack) -> u32 {
    // SAFETY: function contract; `f` is this frame's own stack array
    // and every write below is inside it.
    unsafe {
        let mut passed = 0u32;
        let mut f = [0u8; FRAME_MAX];

        // A valid ICMP echo reply from 10.0.2.2, id 0x4152 seq 1,
        // rebuilt from scratch for each case and then damaged.
        // Every synthetic frame is VALID except for the one thing
        // under test. That matters more than it sounds: the first
        // version built its options case with a checksum covering only
        // 20 bytes, so the frame was refused for a BAD CHECKSUM and
        // the case passed while the IHL rule was not being exercised
        // at all. Verified by injecting the old permissive check and
        // watching the self-test still report 7/7 — a proof that
        // cannot fail is not a proof, which this project has now
        // learned twice.
        let build =
            |f: &mut [u8; FRAME_MAX], src: [u8; 4], ihl_words: u8, frag: u16, total: u16| {
                for b in f.iter_mut() {
                    *b = 0;
                }
                let ihl = ihl_words as usize * 4;
                f[12] = (ETHERTYPE_IPV4 >> 8) as u8;
                f[13] = (ETHERTYPE_IPV4 & 0xFF) as u8;
                f[ETH_HDR] = 0x40 | ihl_words;
                f[ETH_HDR + 2] = (total >> 8) as u8;
                f[ETH_HDR + 3] = (total & 0xFF) as u8;
                f[ETH_HDR + 6] = (frag >> 8) as u8;
                f[ETH_HDR + 7] = (frag & 0xFF) as u8;
                f[ETH_HDR + 8] = IP_TTL;
                f[ETH_HDR + 9] = IP_PROTO_ICMP;
                for i in 0..4 {
                    f[ETH_HDR + 12 + i] = src[i];
                    f[ETH_HDR + 16 + i] = SLIRP_GUEST_IP[i];
                }
                // Options (when ihl > 5) are NOPs, and the checksum covers
                // the header that is actually there.
                for i in IP_HDR..ihl {
                    f[ETH_HDR + i] = 1;
                }
                let ck = checksum(f.as_ptr().add(ETH_HDR), ihl);
                f[ETH_HDR + 10] = (ck >> 8) as u8;
                f[ETH_HDR + 11] = (ck & 0xFF) as u8;
                let ic = ETH_HDR + ihl;
                f[ic] = ICMP_ECHO_REPLY;
                f[ic + 4] = 0x41;
                f[ic + 5] = 0x52;
                f[ic + 7] = 1;
                // The same position-dependent payload a real echo
                // carries. The control case must be genuine in EVERY
                // respect the parser checks or it stops being a
                // control — adding the payload-integrity check
                // immediately failed it, which is the self-test doing
                // its job on itself.
                for i in 0..PING_PAYLOAD {
                    f[ic + ICMP_HDR + i] = b"arenaos!"[i % 8] ^ (i as u8);
                }
                let ck = checksum(f.as_ptr().add(ic), ICMP_HDR + PING_PAYLOAD);
                f[ic + 2] = (ck >> 8) as u8;
                f[ic + 3] = (ck & 0xFF) as u8;
            };
        let total_ok = (IP_HDR + ICMP_HDR + PING_PAYLOAD) as u16;
        let frame_len = ETH_HDR + total_ok as usize;

        // The stack must believe it is waiting for exactly this echo.
        st8.want_icmp = Some((0x4152, 1));
        st8.want_icmp_ip = Some(SLIRP_GATEWAY_IP);

        // 1. OPTIONS (IHL 6) — refused: v1 says fixed 20-byte header.
        //    Otherwise entirely valid: correct checksum over the
        //    24-byte header, ICMP where the header says it is.
        let with_options = (IP_HDR + 4 + ICMP_HDR + PING_PAYLOAD) as u16;
        build(&mut f, SLIRP_GATEWAY_IP, 6, 0, with_options);
        st8.got_icmp = false;
        demux(st8, f.as_ptr(), ETH_HDR + with_options as usize);
        if !st8.got_icmp {
            passed += 1;
        }

        // 2. A FRAGMENT (more-fragments set) — refused: no reassembly.
        let before = st8.rx_fragments;
        build(&mut f, SLIRP_GATEWAY_IP, 5, 0x2000, total_ok);
        st8.got_icmp = false;
        demux(st8, f.as_ptr(), frame_len);
        if !st8.got_icmp && st8.rx_fragments == before + 1 {
            passed += 1;
        }

        // 3. A NONZERO FRAGMENT OFFSET — refused for the same reason.
        let before = st8.rx_fragments;
        build(&mut f, SLIRP_GATEWAY_IP, 5, 0x0001, total_ok);
        st8.got_icmp = false;
        demux(st8, f.as_ptr(), frame_len);
        if !st8.got_icmp && st8.rx_fragments == before + 1 {
            passed += 1;
        }

        // 4. A DECLARED LENGTH LONGER THAN THE FRAME — refused.
        build(&mut f, SLIRP_GATEWAY_IP, 5, 0, total_ok + 8);
        st8.got_icmp = false;
        demux(st8, f.as_ptr(), frame_len);
        if !st8.got_icmp {
            passed += 1;
        }

        // 5. A CORRUPT HEADER CHECKSUM — refused and counted.
        let before = st8.rx_bad_checksum;
        build(&mut f, SLIRP_GATEWAY_IP, 5, 0, total_ok);
        f[ETH_HDR + 10] ^= 0xFF;
        st8.got_icmp = false;
        demux(st8, f.as_ptr(), frame_len);
        if !st8.got_icmp && st8.rx_bad_checksum == before + 1 {
            passed += 1;
        }

        // 6. THE RIGHT ECHO FROM THE WRONG HOST — refused. This is the
        //    one a matching id/seq alone would have accepted.
        let before = st8.rx_wrong_source;
        build(&mut f, [10, 0, 2, 99], 5, 0, total_ok);
        st8.got_icmp = false;
        demux(st8, f.as_ptr(), frame_len);
        if !st8.got_icmp && st8.rx_wrong_source == before + 1 {
            passed += 1;
        }

        // 7. THE CONTROL: the genuine article must still be accepted,
        //    or the six refusals above prove only that nothing works.
        build(&mut f, SLIRP_GATEWAY_IP, 5, 0, total_ok);
        st8.got_icmp = false;
        demux(st8, f.as_ptr(), frame_len);
        if st8.got_icmp {
            passed += 1;
        }

        // Leave no trace: the live counters describe the WIRE.
        st8.want_icmp = None;
        st8.want_icmp_ip = None;
        st8.got_icmp = false;
        st8.rx_frames = 0;
        st8.rx_arp = 0;
        st8.rx_ipv4 = 0;
        st8.rx_dropped = 0;
        st8.rx_bad_checksum = 0;
        st8.rx_fragments = 0;
        st8.rx_wrong_source = 0;
        passed
    }
}

/// Synthetic UDP checks the negative space the wire won't send: a
/// genuine odd-length control, one corrupted payload, and the IPv4
/// zero-checksum exception. An outbound check proves we do not merely
/// accept slirp's reply while still sending unprotected datagrams.
///
/// # Safety
/// As `_start`.
unsafe fn udp_selftest(st8: &mut Stack) -> u32 {
    // SAFETY: all buffers are ours and each checksum length is bounded.
    unsafe {
        let mut passed = 0;
        let payload = *b"abcde";
        let len = build_udp(
            st8,
            [1; 6],
            [10, 0, 2, 3],
            5353,
            53,
            payload.as_ptr(),
            payload.len(),
        );
        let tx_udp = (st8.tx + (ETH_HDR + IP_HDR) as u64) as *const u8;
        let ck = ((*tx_udp.add(6) as u16) << 8) | *tx_udp.add(7) as u16;
        if len == (ETH_HDR + IP_HDR + UDP_HDR + 5) as u64
            && ck != 0
            && udp_checksum(SLIRP_GUEST_IP, [10, 0, 2, 3], tx_udp, UDP_HDR + 5) == 0
        {
            passed += 1;
        }
        let mut rx = [0u8; UDP_HDR + 5];
        rx[..6].copy_from_slice(&[0, 53, 0x14, 0xe9, 0, 13]); // dst 5353
        rx[8..].copy_from_slice(&payload);
        let sum = udp_checksum([10, 0, 2, 3], SLIRP_GUEST_IP, rx.as_ptr(), rx.len());
        rx[6..8].copy_from_slice(&sum.to_be_bytes());
        st8.bindings[0] = Binding {
            live: true,
            port: 5353,
            handle: 1,
            ..NO_BINDING
        };
        udp_in(st8, [10, 0, 2, 3], rx.as_ptr(), rx.len());
        if st8.bindings[0].have && st8.bindings[0].data[..5] == payload {
            passed += 1;
        }
        st8.bindings[0].have = false;
        rx[8] ^= 1; // only the payload is damaged; UDP length stays valid
        let bad = st8.rx_bad_checksum;
        udp_in(st8, [10, 0, 2, 3], rx.as_ptr(), rx.len());
        if !st8.bindings[0].have && st8.rx_bad_checksum == bad + 1 {
            passed += 1;
        }
        rx[6] = 0;
        rx[7] = 0; // legal (but weaker) in IPv4, regardless of payload
        udp_in(st8, [10, 0, 2, 3], rx.as_ptr(), rx.len());
        if st8.bindings[0].have {
            passed += 1;
        }
        st8.bindings[0] = NO_BINDING;
        st8.rx_udp = 0;
        st8.rx_dropped = 0;
        st8.rx_bad_checksum = 0;
        passed
    }
}

/// Send whatever is in the transmit frame, `len` bytes of it.
///
/// # Safety
/// As `_start`.
unsafe fn transmit(_st8: &mut Stack, len: u64) -> u64 {
    // SAFETY: function contract.
    unsafe {
        // Clear the lend slot before copying into it: a CALL does not
        // empty the caller's send-cap slot.
        let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
        if syscall3(SYS_CAP_COPY, SLOT_TX_MASTER, SLOT_TX_LENT, RIGHTS_ALL) < 0 {
            fail(EXIT_SETUP, "the transmit cap copy refused");
        }
        let (status, _) = netd_call(NET_OP_SEND, len, SLOT_TX_LENT, 0);
        if status != NET_S_OK {
            let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
        }
        status
    }
}

/// Resolve an IPv4 address: cache first, then the wire.
///
/// # Safety
/// As `_start`.
unsafe fn resolve(st8: &mut Stack, ip: [u8; 4]) -> Option<[u8; 6]> {
    // SAFETY: function contract.
    unsafe {
        let now = syscall0(SYS_CLOCK_NOW).max(0) as u64;
        if let Some(mac) = st8.lookup(ip, now) {
            st8.hits += 1;
            log_line(|o| {
                o.str("netstackd: cache HIT for ");
                for (i, b) in ip.iter().enumerate() {
                    if i > 0 {
                        o.str(".");
                    }
                    o.u64(*b as u64);
                }
                o.str(" — no frame touched the wire");
            });
            return Some(mac);
        }

        st8.want_arp = Some(ip);
        st8.got_arp = None;
        for attempt in 1..=ARP_TRIES {
            build_request(st8, ip);
            let status = transmit(st8, ARP_FRAME_LEN);
            if status == ARP_S_LINK_DOWN {
                if reattach(st8) {
                    continue;
                }
                st8.want_arp = None;
                return None;
            }
            if status != NET_S_OK {
                st8.want_arp = None;
                return None;
            }
            st8.wire_requests += 1;

            // Wait for the DEMULTIPLEXER to recognise our answer,
            // bounded by netd's deadline (a client blocked in
            // SYS_IPC_CALL cannot bound itself — ADR-0029's erratum).
            while st8.got_arp.is_none() {
                if !pump(st8, REPLY_TIMEOUT_US) {
                    break;
                }
            }
            if let Some(mac) = st8.got_arp {
                st8.want_arp = None;
                let now = syscall0(SYS_CLOCK_NOW).max(0) as u64;
                st8.insert(ip, mac, now);
                log_line(|o| {
                    o.str("netstackd: resolved ");
                    for (i, b) in ip.iter().enumerate() {
                        if i > 0 {
                            o.str(".");
                        }
                        o.u64(*b as u64);
                    }
                    o.str(" → ");
                    for (i, b) in mac.iter().enumerate() {
                        if i > 0 {
                            o.str(":");
                        }
                        o.hex2(*b);
                    }
                    o.str(" on attempt ");
                    o.u64(attempt as u64);
                    o.str(" (cached for 30s)");
                });
                return Some(mac);
            }
        }
        st8.want_arp = None;
        log("netstackd: no ARP reply after every retry — reporting unreachable, not guessing");
        None
    }
}

/// Ping: resolve if needed, send an ICMP echo, time the answer.
///
/// Returns the round-trip time in microseconds, measured with the
/// machine's monotonic clock rather than counted in retries.
///
/// # Safety
/// As `_start`.
unsafe fn ping(st8: &mut Stack, ip: [u8; 4]) -> Result<u64, u64> {
    // SAFETY: function contract.
    unsafe {
        // Layering, visible: an echo needs a destination MAC, so a
        // ping to an unresolved address is an ARP exchange first.
        let Some(mac) = resolve(st8, ip) else {
            return Err(ARP_S_UNREACHABLE);
        };

        let id = 0x4152; // 'AR' — this stack's echo identifier
        for _ in 0..PING_TRIES {
            st8.ping_seq = st8.ping_seq.wrapping_add(1);
            let seq = st8.ping_seq;
            build_echo(st8, mac, ip, id, seq);
            st8.want_icmp = Some((id, seq));
            st8.want_icmp_ip = Some(ip);
            st8.got_icmp = false;
            let sent_us = syscall0(SYS_CLOCK_NOW).max(0) as u64;
            let status = transmit(st8, PING_FRAME_LEN);
            if status == ARP_S_LINK_DOWN {
                if reattach(st8) {
                    continue;
                }
                st8.want_icmp = None;
                st8.want_icmp_ip = None;
                return Err(ARP_S_LINK_DOWN);
            }
            if status != NET_S_OK {
                st8.want_icmp = None;
                st8.want_icmp_ip = None;
                return Err(ARP_S_LINK_DOWN);
            }
            while !st8.got_icmp {
                if !pump(st8, PING_TIMEOUT_US) {
                    break;
                }
            }
            if st8.got_icmp {
                let rtt = (syscall0(SYS_CLOCK_NOW).max(0) as u64).saturating_sub(sent_us);
                st8.want_icmp = None;
                st8.want_icmp_ip = None;
                log_line(|o| {
                    o.str("netstackd: echo reply from ");
                    for (i, b) in ip.iter().enumerate() {
                        if i > 0 {
                            o.str(".");
                        }
                        o.u64(*b as u64);
                    }
                    o.str(" seq ");
                    o.u64(seq as u64);
                    o.str(" in ");
                    o.u64(rtt);
                    o.str("us — identifier, sequence and all ");
                    o.u64(PING_PAYLOAD as u64);
                    o.str(" payload bytes matched across the chunked receive");
                });
                return Ok(rtt);
            }
        }
        st8.want_icmp = None;
        log(
            "netstackd: the host resolved but never answered an echo — NO_REPLY, which is not the same as unreachable",
        );
        Err(ICMP_S_NO_REPLY)
    }
}

/// Lay out an ICMP echo request: Ethernet, then IPv4, then ICMP, with
/// both checksums computed over what was actually written.
///
/// # Safety
/// `st8.tx` is this image's own mapped frame.
unsafe fn build_echo(st8: &Stack, mac: [u8; 6], ip: [u8; 4], id: u16, seq: u16) {
    // SAFETY: method contract; the frame is one mapped page and every
    // offset below is inside PING_FRAME_LEN.
    unsafe {
        let base = st8.tx;
        let put = |off: usize, b: u8| *((base + off as u64) as *mut u8) = b;
        // Ethernet
        for (i, (dst, src)) in mac.iter().zip(st8.mac.iter()).enumerate() {
            put(i, *dst);
            put(6 + i, *src);
        }
        put(12, (ETHERTYPE_IPV4 >> 8) as u8);
        put(13, (ETHERTYPE_IPV4 & 0xFF) as u8);
        // IPv4
        let total = (IP_HDR + ICMP_HDR + PING_PAYLOAD) as u16;
        put(ETH_HDR, 0x45); // version 4, IHL 5 (no options)
        put(ETH_HDR + 1, 0); // DSCP/ECN
        put(ETH_HDR + 2, (total >> 8) as u8);
        put(ETH_HDR + 3, (total & 0xFF) as u8);
        put(ETH_HDR + 4, 0);
        put(ETH_HDR + 5, 0); // identification
        put(ETH_HDR + 6, 0x40); // don't fragment
        put(ETH_HDR + 7, 0);
        put(ETH_HDR + 8, IP_TTL);
        put(ETH_HDR + 9, IP_PROTO_ICMP);
        put(ETH_HDR + 10, 0);
        put(ETH_HDR + 11, 0); // checksum, filled below
        for i in 0..4 {
            put(ETH_HDR + 12 + i, SLIRP_GUEST_IP[i]);
            put(ETH_HDR + 16 + i, ip[i]);
        }
        let ck = checksum((base + ETH_HDR as u64) as *const u8, IP_HDR);
        put(ETH_HDR + 10, (ck >> 8) as u8);
        put(ETH_HDR + 11, (ck & 0xFF) as u8);
        // ICMP
        let ic = ETH_HDR + IP_HDR;
        put(ic, ICMP_ECHO_REQUEST);
        put(ic + 1, 0);
        put(ic + 2, 0);
        put(ic + 3, 0); // checksum, filled below
        put(ic + 4, (id >> 8) as u8);
        put(ic + 5, (id & 0xFF) as u8);
        put(ic + 6, (seq >> 8) as u8);
        put(ic + 7, (seq & 0xFF) as u8);
        // A repeating, POSITION-DEPENDENT pattern. Constant bytes
        // would let a reassembly bug that duplicated or reordered a
        // chunk pass unnoticed; this way the echoed payload only
        // matches if every chunk came back in the right place.
        for i in 0..PING_PAYLOAD {
            put(ic + ICMP_HDR + i, b"arenaos!"[i % 8] ^ (i as u8));
        }
        let ck = checksum((base + ic as u64) as *const u8, ICMP_HDR + PING_PAYLOAD);
        put(ic + 2, (ck >> 8) as u8);
        put(ic + 3, (ck & 0xFF) as u8);
    }
}

/// Lay out a 42-byte ARP request at exact wire offsets.
///
/// # Safety
/// `st8.tx` is this image's own mapped frame.
unsafe fn build_request(st8: &Stack, ip: [u8; 4]) {
    // SAFETY: method contract; the frame is one mapped page.
    unsafe {
        let put = |off: u64, b: u8| *((st8.tx + off) as *mut u8) = b;
        for i in 0..6u64 {
            put(i, 0xFF); // destination: broadcast
            put(32 + i, 0); // target MAC: the question
        }
        for (i, b) in st8.mac.iter().enumerate() {
            put(6 + i as u64, *b);
            put(22 + i as u64, *b);
        }
        put(12, (ETHERTYPE_ARP >> 8) as u8);
        put(13, (ETHERTYPE_ARP & 0xFF) as u8);
        put(14, 0x00);
        put(15, 0x01); // htype: Ethernet
        put(16, 0x08);
        put(17, 0x00); // ptype: IPv4
        put(18, 6);
        put(19, 4);
        put(20, (ARP_OP_REQUEST >> 8) as u8);
        put(21, (ARP_OP_REQUEST & 0xFF) as u8);
        for i in 0..4usize {
            put(28 + i as u64, SLIRP_GUEST_IP[i]);
            put(38 + i as u64, ip[i]);
        }
    }
}

/// Is this frame an ARP reply for `want`? Returns the sender's MAC.
///
/// Every field is checked at its wire offset: a stack that accepts
/// whatever arrives is not parsing, it is hoping.
///
/// # Safety
/// `bytes` points at `len` readable bytes of this image's memory.
unsafe fn parse_reply(bytes: *const u8, len: usize, want: [u8; 4]) -> Option<[u8; 6]> {
    if len < ARP_FRAME_LEN as usize {
        return None;
    }
    // SAFETY: caller contract; all offsets are below ARP_FRAME_LEN.
    unsafe {
        let at = |i: usize| *bytes.add(i);
        if at(12) != (ETHERTYPE_ARP >> 8) as u8 || at(13) != (ETHERTYPE_ARP & 0xFF) as u8 {
            return None;
        }
        if at(20) != (ARP_OP_REPLY >> 8) as u8 || at(21) != (ARP_OP_REPLY & 0xFF) as u8 {
            return None;
        }
        // The sender must be the address we asked about.
        for (i, w) in want.iter().enumerate() {
            if at(28 + i) != *w {
                return None;
            }
        }
        let mut mac = [0u8; 6];
        for (i, m) in mac.iter_mut().enumerate() {
            *m = at(22 + i);
        }
        // An all-zero MAC is not an answer.
        if mac.iter().all(|&b| b == 0) {
            return None;
        }
        Some(mac)
    }
}

/// # Safety
/// A reply is legal exactly once per received call.
unsafe fn reply(status: u64, w1: u64, _w2: u64) {
    // SAFETY: wrapper contract.
    let r = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, status, w1, CAP_NONE, 0) };
    if r < 0 {
        fail(EXIT_REPLY, "the reply was refused");
    }
}

fn log(s: &str) {
    log_line(|o| o.str(s));
}

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("netstackd: FAIL: ");
        o.str(what);
        o.str(" — exiting ");
        o.u64(code);
    });
    // SAFETY: thread_exit diverges.
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
