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

const SLOT_NETD: u64 = 0;
const SLOT_EP: u64 = 1;
/// A notification of its own (M7.1b): what the stack backs off on
/// while the supervisor brings a dead driver back. Waiting needs
/// something to wait ON, and spinning on the clock is the polling
/// Phase 7 forbids. GRANTED, so it sits with the other grants at the
/// bottom of the space — the slots this program fills itself start
/// after them.
const SLOT_NOTIF: u64 = 2;
const SLOT_TX: u64 = 3;
/// The MASTER lend copy, taken once before the frame is mapped.
///
/// `SYS_MAP_MEMORY` CONSUMES the cap it maps (ADR-0021: ownership
/// moves into the address space), so after mapping there is no
/// original left to copy from. rngd learned this once and netstackd
/// promptly learned it again: a per-send copy taken from slot 2 after
/// the map is a copy of nothing. The master is lent, never mapped,
/// and every send copies from IT — because each send CONSUMES its
/// copy on the way to the driver.
const SLOT_TX_MASTER: u64 = 4;
/// The per-send lend copy, consumed by the call that carries it.
const SLOT_TX_LENT: u64 = 5;

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
const PING_PAYLOAD: usize = 8;
const PING_FRAME_LEN: u64 = (ETH_HDR + IP_HDR + ICMP_HDR + PING_PAYLOAD) as u64;

const IP_PROTO_ICMP: u8 = 1;
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
            want_arp: None,
            got_arp: None,
            want_icmp: None,
            want_icmp_ip: None,
            got_icmp: false,
            ping_seq: 0,
        };
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
                        o.str(" bad checksum");
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
        let mut frame = [0u64; MSG_BYTES / 8];
        let (status, flen) =
            netd_call(NET_OP_RECV, timeout_us, CAP_NONE, frame.as_mut_ptr() as u64);
        if status == NET_S_TIMEOUT {
            st8.timeouts += 1;
            return false;
        }
        if status == ARP_S_LINK_DOWN {
            return reattach(st8);
        }
        if status != NET_S_OK {
            return false;
        }
        st8.rx_frames += 1;
        demux(st8, frame.as_ptr() as *const u8, flen as usize);
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
        if at(9) != IP_PROTO_ICMP {
            st8.rx_dropped += 1;
            return;
        }
        let mut src = [0u8; 4];
        for (i, b) in src.iter_mut().enumerate() {
            *b = at(12 + i);
        }
        icmp_in(st8, src, ip.add(ihl), total - ihl);
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
                st8.got_icmp = true;
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
        let mut f = [0u8; 64];

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
        let build = |f: &mut [u8; 64], src: [u8; 4], ihl_words: u8, frag: u16, total: u16| {
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
                    o.str("us (identifier and sequence both matched)");
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
        for i in 0..PING_PAYLOAD {
            put(ic + ICMP_HDR + i, b"arenaos!"[i]);
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
