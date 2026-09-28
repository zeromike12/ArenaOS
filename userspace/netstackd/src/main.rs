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
//! **What v1 does.** ARP over IPv4, and nothing else: resolve an IPv4
//! address to a MAC by putting a real request on the wire, cache the
//! answer with a TTL, and hand cached answers back without touching
//! the wire again. That is the smallest slice with a real protocol
//! and a real cache, and it exercises every part of the boundary the
//! rest of Phase 7 will use.
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
const ARP_OP_REQUEST: u16 = 1;
const ARP_OP_REPLY: u16 = 2;

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
        };
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
                        o.str(" re-attach(es)");
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

        for attempt in 1..=ARP_TRIES {
            build_request(st8, ip);
            // Clear the lend slot before copying into it. A CALL does
            // not empty the caller's send-cap slot — the receiver
            // gets its own installed copy — so the second resolve
            // found slot 4 still occupied and the copy was refused.
            // Destroying a lent cap frees nothing (ADR-0022): it only
            // drops this process's reference.
            let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
            if syscall3(SYS_CAP_COPY, SLOT_TX_MASTER, SLOT_TX_LENT, RIGHTS_ALL) < 0 {
                fail(EXIT_SETUP, "the transmit cap copy refused");
            }
            let (status, _) = netd_call(NET_OP_SEND, ARP_FRAME_LEN, SLOT_TX_LENT, 0);
            if status == ARP_S_LINK_DOWN {
                // The driver died holding this request. Re-establish
                // and ask again — safe HERE and only here, because an
                // ARP request is a broadcast query and repeating it
                // costs nothing but a packet.
                let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
                if reattach(st8) {
                    continue;
                }
                return None;
            }
            if status != NET_S_OK {
                let _ = syscall1(SYS_CAP_DESTROY, SLOT_TX_LENT);
                return None;
            }
            st8.wire_requests += 1;

            // Wait for an answer, BOUNDED — netd enforces the
            // deadline because we cannot (ADR-0029's erratum).
            let mut frame = [0u64; MSG_BYTES / 8];
            loop {
                let (status, flen) = netd_call(
                    NET_OP_RECV,
                    REPLY_TIMEOUT_US,
                    CAP_NONE,
                    frame.as_mut_ptr() as u64,
                );
                if status == NET_S_TIMEOUT {
                    st8.timeouts += 1;
                    break; // retry, or give up after ARP_TRIES
                }
                if status == ARP_S_LINK_DOWN {
                    if reattach(st8) {
                        break; // re-ask from the top of the retry loop
                    }
                    return None;
                }
                if status != NET_S_OK {
                    return None;
                }
                let bytes = frame.as_ptr() as *const u8;
                if let Some(mac) = parse_reply(bytes, flen as usize, ip) {
                    st8.replies_seen += 1;
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
                // Some other frame: v1 has no other protocol to give
                // it to, so it is dropped and the wait continues
                // inside the same deadline.
            }
        }
        log("netstackd: no ARP reply after every retry — reporting unreachable, not guessing");
        None
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
