//! The shared userspace ABI surface (M5.2/M5.3, ADR-0022/0023) — one
//! file, included by EVERY service crate binary (`arena-storaged`,
//! `blktest`, `fsd`, `fstest`) through `#[path = "../../abi.rs"]`, so
//! both sides of every wire protocol speak from the same frozen
//! definitions: syscall registry, IPC v1.1 inline-message helpers,
//! block protocol v1.1, and the filesystem-service protocol. No libc,
//! no allocator: fixed buffers, volatile MMIO/ring accessors, and the
//! shell's chunked-console discipline.
//!
//! ABI v1 (ADR-0017): RAX = call number, args RDI/RSI/RDX/R10/R8/R9;
//! the kernel preserves only RBX/RBP/R12–R15, so every stub declares
//! the documented caller-saved set clobbered. Returns: negative = typed
//! status, positive = payload, 0 = OK where a call has no payload.

#![allow(dead_code)] // the two binaries use overlapping subsets

// ---- the frozen syscall registry (numbers mirror kernel syscall.rs) --------

pub const SYS_DEBUG_WRITE: u64 = 1;
pub const SYS_THREAD_EXIT: u64 = 2;
pub const SYS_IPC_CALL: u64 = 7;
pub const SYS_IPC_RECV: u64 = 8;
pub const SYS_IPC_REPLY: u64 = 9;
pub const SYS_NOTIFY: u64 = 10;
pub const SYS_WAIT: u64 = 11;
pub const SYS_SPAWN: u64 = 12;
pub const SYS_CONSOLE_READ: u64 = 13;
pub const SYS_PROC_LIST: u64 = 14;
pub const SYS_SHUTDOWN: u64 = 15;
pub const SYS_ALLOC_FRAME: u64 = 16;
pub const SYS_MAP_MEMORY: u64 = 17;
pub const SYS_IRQ_RELAY: u64 = 18;
pub const SYS_CAP_PHYS: u64 = 19;
pub const SYS_CAP_DESTROY: u64 = 20;
pub const SYS_CAP_COPY: u64 = 21;
pub const SYS_DEV_INFO: u64 = 22;
pub const SYS_CONSOLE_PUSH: u64 = 23;
pub const SYS_CONSOLE_ATTACH: u64 = 24;
pub const SYS_CONSOLE_PULL: u64 = 25;
pub const SYS_CLOCK_NOW: u64 = 26;
pub const SYS_TIMER_ARM: u64 = 27;
pub const SYS_TIMER_CANCEL: u64 = 28;
/// Inspect an already-held cap into [kind, object, rights] (24 bytes).
pub const SYS_CAP_DESCRIBE: u64 = 29;
/// Finish a child by Process-cap slot: 0=reap exited, 1=explicit stop/reap.
pub const SYS_PROC_FINISH: u64 = 30;

// ---- cap/IPC constants (mirror kernel cap.rs / ipc.rs) ----------------------

pub const CAP_NONE: u64 = u64::MAX;
pub const RIGHTS_READ: u64 = 1 << 0;
pub const RIGHTS_WRITE: u64 = 1 << 1;
pub const RIGHTS_COPY: u64 = 1 << 2;
pub const RIGHTS_DESTROY: u64 = 1 << 3;
pub const RIGHTS_ALL: u64 = RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY;

// ---- IPC v1.1 inline messages (M5.3, ADR-0023) -------------------------------
//
// CALL/RECV/REPLY take an OPTIONAL trailing pointer to a 64-byte
// buffer: the kernel snapshots it from the CALLER/REPLIER (owner
// context), rides it in the call slot, and hands it to the other side.
// A null pointer means "no inline message" (v1.0 behavior). Payloads
// are opaque bytes — the FS protocol below defines its layouts.

pub const MSG_BYTES: usize = 64;

// ---- the block protocol (ADR-0022, extended v1.1 by ADR-0023) ----------------
//
// Request words:  w0 = sector (512-byte units),
//                 w1 = op | (buf_offset << 8) — buf_offset selects the
//                 sector's landing spot INSIDE the caller's lent 4 KiB
//                 frame, so one frame can stage up to 8 sectors and
//                 forwarded caps keep DMA'ing the client's own memory.
//                 The driver MUST refuse offsets with
//                 buf_offset + 512 > 4096 (a wild offset could aim the
//                 device at an adjacent frame).
// Reply words:    w0 = virtio status byte (0 = OK), w1 = device-written
//                 byte count from the used ring (informational).
// The caller's buffer travels as a LENT Untyped cap attached to the
// call — the driver learns its phys through SYS_CAP_PHYS and the device
// DMAs the caller's own frame (zero-copy). Exactly one sector per
// request in v1; the buffer cap must name at least SECTOR_BYTES.

pub const OP_READ: u64 = 0;
pub const OP_WRITE: u64 = 1;
/// The poison request: reply, then exit cleanly (a driver process with
/// parked threads must never be destroyed out from under them).
pub const OP_SHUTDOWN: u64 = 2;
pub const SECTOR_BYTES: usize = 512;
/// Every lent buffer frame is one 4 KiB page (Untyped cap granularity).
pub const BLOCK_FRAME_BYTES: u64 = 4096;

/// Pack a block-protocol request word 1 (op + in-frame buffer offset).
pub fn block_req_w1(op: u64, buf_offset: u64) -> u64 {
    op | (buf_offset << 8)
}

// ---- the filesystem-service protocol (ADR-0023) --------------------------------
//
// fsd owns AFS1 (extent data + CoW/transactional metadata) and serves
// it on its own endpoint. Client data buffers travel the same way as
// block requests: a LENT Untyped cap attached to the call, which fsd
// FORWARDS to storaged — the device DMAs straight between the disk and
// the CLIENT's frame (zero-copy end to end; fsd never maps it — lent
// caps cannot be mapped by design).
//
// Request word 0 is the op; word 1 depends on the op:
//   CREATE / OPEN  w1 ignored; msg64 = name bytes (<= FS_NAME_MAX)
//   READ / WRITE   w1 = fh | (file_offset << 8); msg64 [0..8] = length
//                  (bytes, <= 3584); send cap = client's LENT frame.
//                  v1 WRITE must start at or below EOF (gap/sparse
//                  writes answer FS_ERR_RANGE — the extent mapping is
//                  cumulative and only appends stay honest)
//   CLOSE          w1 = fh
//   UNLINK         w1 ignored; msg64 = name bytes. Transactional
//                  delete (M5.4): the object and its extent chain die
//                  with two-generation delay; an OPEN file answers
//                  FS_ERR_BUSY (v1 has no unlink-at-last-close)
//   LS             w1 = cursor (0 to start); msg64 OUT = dirent:
//                  [0..4] next cursor (FS_CURSOR_END = done),
//                  [4..12] size, [12..16] name length, [16..48] name
//   SHUTDOWN       reply, then fsd exits (same discipline as storaged)
// Reply word 0 is FS_OK or a negative FS_ERR_* status; word 1:
//   CREATE/OPEN -> file handle, READ/WRITE -> byte count,
//   SHUTDOWN -> fsd's lifetime disk-operation count, else 0.

pub const FS_OP_CREATE: u64 = 1;
pub const FS_OP_OPEN: u64 = 2;
pub const FS_OP_READ: u64 = 3;
pub const FS_OP_WRITE: u64 = 4;
pub const FS_OP_CLOSE: u64 = 5;
pub const FS_OP_LS: u64 = 6;
pub const FS_OP_SHUTDOWN: u64 = 7;
pub const FS_OP_UNLINK: u64 = 8;

pub const FS_OK: u64 = 0;
pub const FS_ERR_NOT_FOUND: u64 = (-1i64) as u64;
pub const FS_ERR_EXISTS: u64 = (-2i64) as u64;
pub const FS_ERR_BAD_FH: u64 = (-3i64) as u64;
pub const FS_ERR_TABLE_FULL: u64 = (-4i64) as u64;
pub const FS_ERR_IO: u64 = (-5i64) as u64;
pub const FS_ERR_NO_SPACE: u64 = (-6i64) as u64;
pub const FS_ERR_BAD_NAME: u64 = (-7i64) as u64;
pub const FS_ERR_CORRUPT: u64 = (-8i64) as u64;
pub const FS_ERR_RANGE: u64 = (-9i64) as u64;
pub const FS_ERR_BAD_OP: u64 = (-10i64) as u64;
pub const FS_ERR_BUSY: u64 = (-11i64) as u64;

pub const FS_NAME_MAX: usize = 32;

/// The suite's scratch-disk geometry in 512-byte sectors — mirrors
/// `tools/arena_env.py` (`SCRATCH_MIB = 8`). blktest's raw-block cycle
/// claims the LAST sector: the host-side mkfs (ADR-0023) puts AFS1's
/// metadata in sectors 0..10 and the suite's fsd allocates upward from
/// 11, so a raw write there cannot touch filesystem state.
pub const SCRATCH_TOTAL_SECTORS: u64 = 16384;

/// The deterministic test pattern shared by blktest and fstest — and
/// mirrored byte-for-byte by `tools/test_m5.py`'s on-disk verification
/// (the host parses the real committed sectors after boot).
pub fn pattern_byte(i: usize) -> u8 {
    (i as u8).wrapping_mul(31).wrapping_add(0x5A) ^ ((i >> 3) as u8)
}
pub const FS_CURSOR_END: u32 = 0xFFFF_FFFF;
/// Largest single READ/WRITE transfer: one 4 KiB frame minus the
/// largest in-frame offset the protocol can express (512 * 1 = one
/// sector alignment slack kept for the final partial sector).
pub const FS_XFER_MAX: u64 = 3584;

/// Pack the FS READ/WRITE request word 1 (handle + file offset).
pub fn fs_rw_w1(fh: u64, file_offset: u64) -> u64 {
    fh | (file_offset << 8)
}
/// virtio-blk status values (virtio 1.0 §5.2.6.1).
pub const VIRTIO_BLK_S_OK: u64 = 0;
pub const VIRTIO_BLK_S_IOERR: u64 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u64 = 2;

// ---- the network-service protocol (M6.1, ADR-0024) ---------------------------
//
// netd owns the virtio-net device and serves it on its own endpoint,
// LINK-LAYER ONLY: raw Ethernet frames in, raw Ethernet frames out — no
// protocols of any kind (those are the callers' business until Phase 7
// gives them a home). The caller's frames travel as LENT Untyped caps
// attached to the call — the block discipline verbatim: netd learns the
// phys through SYS_CAP_PHYS and the device DMAs the CALLER's own page
// (zero-copy TX, chained behind netd's own virtio header; RX v1 copies
// the held frame into the caller's buffer — the documented ADR-0024 v1
// simplification).
//
// Request word 0 depends on the op; word 1 = op:
//   SEND      w0 = frame length (NET_FRAME_MIN..=NET_FRAME_MAX); send
//             cap = the caller's LENT frame, frame bytes at offset 0
//   RECV      w0 = 0, no send cap; blocks until netd holds an
//             undelivered received frame, which returns in the REPLY's
//             inline message (the fstest-LS pattern). v1 delivers
//             frames up to MSG_BYTES; a longer held frame is dropped
//             with a typed refusal and an honest log — full-frame
//             delivery needs the Phase 7 buffer-handoff design
//             (ADR-0024 records the limitation)
//   MAC       no cap (a landed cap is refused, not leaked)
//   SHUTDOWN  poison: netd replies FIRST, then exits by its own hand
// Reply word 0 is NET_S_*; word 1:
//   SEND -> bytes transmitted, RECV -> frame length in the inline
//   message, MAC -> the six MAC bytes packed little-endian,
//   SHUTDOWN -> netd's lifetime interrupt-delivered completion count.

pub const NET_OP_SHUTDOWN: u64 = 0;
pub const NET_OP_SEND: u64 = 1;
/// RECV(w0 = timeout_us): hand back the next received frame inline.
///
/// BOUNDED since M7.1 (ADR-0030). It used to block until a frame
/// arrived, full stop — which is unusable for a protocol: a lost ARP
/// reply would park the caller forever, and ADR-0029's erratum
/// records why a timer cannot rescue a thread blocked in an IPC call.
/// So the deadline is enforced by the SERVER, which is the only party
/// that can: netd arms a timer on its own notification and waits for
/// "a frame arrived OR the deadline passed", then answers either way.
/// A timeout_us of 0 means "poll" — answer immediately with whatever
/// is already held.
pub const NET_OP_RECV: u64 = 2;
pub const NET_OP_MAC: u64 = 3;
/// RECV_CHUNK(w0 = byte offset): continue reading the frame that the
/// last `RECV` staged (M7.3, ADR-0032).
///
/// A frame larger than one IPC message used to be dropped, which made
/// every protocol above this driver a protocol for small packets: a
/// DNS answer does not fit in 64 bytes. `RECV` now stages the frame
/// and returns its FULL length with the first chunk; the caller reads
/// the rest by offset, and the buffer returns to the receive ring
/// when the last chunk is taken.
pub const NET_OP_RECV_CHUNK: u64 = 4;

/// Bytes of frame carried by one RECV or RECV_CHUNK reply.
pub const NET_CHUNK: usize = MSG_BYTES;

pub const NET_S_OK: u64 = 0;
pub const NET_S_BAD_OP: u64 = (-1i64) as u64;
pub const NET_S_BAD_LEN: u64 = (-2i64) as u64;
pub const NET_S_NO_BUF: u64 = (-3i64) as u64;
/// No frame arrived before the caller's deadline (M7.1). Not an
/// error: the wire is allowed to be silent, and a protocol that asked
/// "wait up to 200 ms" needs the answer "nothing came" more than it
/// needs to be blocked forever. See `NET_OP_RECV`.
pub const NET_S_TIMEOUT: u64 = (-4i64) as u64;

/// The badge netd arms its own deadline timer with, on its own
/// interrupt notification. Distinct bit from the two MSI-X relay
/// badges (badges are BITS — ADR-0028).
pub const NET_BADGE_DEADLINE: u64 = 1 << 20;

/// Largest Ethernet frame NET_SEND accepts (no jumbos in v1).
pub const NET_FRAME_MAX: u64 = 1514;
/// Smallest legal Ethernet frame (a bare header — nothing shorter can
/// be addressing anything).
pub const NET_FRAME_MIN: u64 = 14;

// ---- the entropy-service protocol (M6.2, ADR-0025) ---------------------------
//
// rngd (registry image 8) serves device entropy from virtio-rng's single
// request queue. A GET LENDs the caller's frame: rngd points ONE
// device-writable descriptor at the frame's PHYS (through SYS_CAP_PHYS,
// never a message word) and the device DMAs entropy directly into the
// caller's own page — zero-copy fill. The completion's used-ring length is
// the device's own count of bytes written; rngd returns it, and the caller
// reads its page (already mapped RW in its own space).
//
//   SHUTDOWN  no cap; reply w1 = the completions served by interrupt.
//   GET       w0 = requested bytes (1..=RNG_DRAW_MAX); send cap = the
//             LENT frame (the device-writable target). Reply w1 = the
//             bytes the device actually wrote.
//
// Reply word 0 is RNG_S_*; word 1 as above.
pub const RNG_OP_SHUTDOWN: u64 = 0;
pub const RNG_OP_GET: u64 = 1;

pub const RNG_S_OK: u64 = 0;
pub const RNG_S_BAD_OP: u64 = (-1i64) as u64;
pub const RNG_S_BAD_LEN: u64 = (-2i64) as u64;
pub const RNG_S_NO_BUF: u64 = (-3i64) as u64;

/// The largest single draw RNG_GET accepts — one frame: the caller
/// LENDs a 4 KiB page and the descriptor covers at most all of it.
pub const RNG_DRAW_MAX: u64 = 4096;

// ---- the input protocol (M6.3, ADR-0026) ------------------------------------
//
// inputd serves DECODED key bytes, not raw evdev events: the keymap is
// the driver's business, and every client speaks the same ASCII the
// console line discipline does.
//
// Request words:  w0 = op, w1 = op-specific
//   READ      w1 = max bytes wanted (1..=INPUT_READ_MAX). Blocks until
//             at least one key is buffered; replies with the count in
//             word 1 and the bytes in the reply's INLINE MESSAGE.
//   SHUTDOWN  the poison request: inputd replies (word 1 = the count of
//             interrupt-delivered event batches) and exits.
// Reply word 0 is INPUT_S_*.
pub const INPUT_OP_SHUTDOWN: u64 = 0;
pub const INPUT_OP_READ: u64 = 1;

pub const INPUT_S_OK: u64 = 0;
pub const INPUT_S_BAD_OP: u64 = (-1i64) as u64;
pub const INPUT_S_BAD_LEN: u64 = (-2i64) as u64;
/// A READ was abandoned because nobody typed: the service's SPAWNER
/// decided to stop waiting (see [`INPUT_BADGE_GIVE_UP`]). Not an
/// error — an honest "there is nothing to give you".
pub const INPUT_S_NO_KEYS: u64 = (-3i64) as u64;

/// A NOTIFICATION BADGE IS A BIT, NOT A NUMBER. `ipc::notify` merges
/// badges with OR and `SYS_WAIT` returns the union, so every word that
/// can arrive on the same notification must occupy a DISJOINT bit and
/// be tested with `&`. Words that merely differ numerically alias:
/// M6.4 shipped 0x7401/0x7411/0x7402 for a driver's receive, transmit,
/// and give-up, and `badge & 0x7402` is nonzero for all three — a
/// transmit completion read as "the supervisor called the wait off",
/// about six boots in a hundred (whenever the real answer had not
/// already arrived in the same wake). Single bits, always.
///
/// The relay notification carries the device's interrupt badge — and
/// ONE other bit, by contract: whoever spawned inputd (and therefore
/// owns the write side of that notification) may send
/// `INPUT_BADGE_GIVE_UP` to abandon a pending READ. A keyboard driver
/// cannot time out by itself — a key that never comes is
/// indistinguishable from one that comes later — so the decision to
/// stop waiting belongs to the supervisor that knows whether anyone is
/// expected to type. The m6 suite uses it so that a boot with a
/// keyboard attached and nobody at it reports an honest SKIP instead of
/// hanging (ADR-0026).
pub const INPUT_BADGE_GIVE_UP: u64 = 1 << 19;

/// Largest READ the service answers in one reply — the inline message
/// is MSG_BYTES (64) and the count rides in a register, so the whole
/// payload fits with room to spare.
pub const INPUT_READ_MAX: u64 = 48;

/// Largest byte run one `SYS_CONSOLE_PUSH` accepts (mirrors the
/// kernel's own bound: a keyboard produces bytes one keystroke at a
/// time, so this is generous).
pub const CONSOLE_PUSH_MAX: u64 = 64;

/// Largest run one `SYS_CONSOLE_PULL` returns (the kernel's own bound).
pub const CONSOLE_PULL_MAX: u64 = 256;

// ---- the console-channel protocol (M6.4, ADR-0027) --------------------------
//
// consoled moves BYTES between a virtio-console port and the machine's
// console. In production it needs no protocol at all — the kernel's
// line discipline and output mirror are its two endpoints. The
// protocol exists for the m6 suite's instance, which is denied both
// console capabilities and therefore serves the port over IPC instead:
// the same driver, the same queues, an observable boundary.
//
// Request words:  w0 = op, w1 = op-specific
//   WRITE     w1 = byte count (1..=CONSOLE_MSG_MAX) in the INLINE
//             message; the driver sends them out the port's transmit
//             queue and replies when the DEVICE has taken them.
//   READ      w1 = max bytes wanted (1..=CONSOLE_MSG_MAX). Blocks
//             until the host has sent something; replies with the
//             count in word 1 and the bytes inline.
//   SHUTDOWN  the poison request: consoled replies (word 1 = bytes
//             sent, word 2 = bytes received) and exits.
// Reply word 0 is CONSOLE_S_*.
pub const CONSOLE_OP_SHUTDOWN: u64 = 0;
pub const CONSOLE_OP_WRITE: u64 = 1;
pub const CONSOLE_OP_READ: u64 = 2;

pub const CONSOLE_S_OK: u64 = 0;
pub const CONSOLE_S_BAD_OP: u64 = (-1i64) as u64;
pub const CONSOLE_S_BAD_LEN: u64 = (-2i64) as u64;
/// A READ was abandoned because nothing was ever sent from the host:
/// the spawner called the wait off (see [`CONSOLE_BADGE_GIVE_UP`]).
/// The same honest answer `INPUT_S_NO_KEYS` gives for a keyboard
/// nobody types on — a port with nobody attached to its far end is
/// the same shape of nothing.
pub const CONSOLE_S_NO_DATA: u64 = (-3i64) as u64;

/// Largest payload one console request or reply carries: the inline
/// message is MSG_BYTES (64) and the count rides in a register.
pub const CONSOLE_MSG_MAX: u64 = 48;

/// The spawner's give-up word on consoled's notification (ADR-0026's
/// pattern, second use — see [`INPUT_BADGE_GIVE_UP`]).
pub const CONSOLE_BADGE_GIVE_UP: u64 = 1 << 19;

// ---- the network stack protocol (M7.1, ADR-0030) ----------------------------
//
// `netstackd` owns protocol STATE; `netd` owns the device. The split
// is deliberate and load-bearing (ROADMAP Phase 7, decision 1): a
// driver's job is to survive its device and be restartable, a stack's
// job is to hold state across time, and mixing them means a wedged
// NIC takes every connection with it.
//
// Request words: w0 = op-specific, w1 = op.
//   RESOLVE   w0 = IPv4 address, big-endian as it appears on the wire.
//             Replies with the MAC packed little-endian into word 1
//             (6 bytes, low byte first — the same packing NET_OP_MAC
//             uses), or a typed failure.
//   STATS     replies with cache hits in word 1 and the number of ARP
//             requests actually PUT ON THE WIRE in word 2. The second
//             number is how a cache is proven: a hit must not move it.
//   SHUTDOWN  the poison request.
// Reply word 0 is ARP_S_*.
pub const ARP_OP_SHUTDOWN: u64 = 0;
pub const ARP_OP_RESOLVE: u64 = 1;
pub const ARP_OP_STATS: u64 = 2;
/// PING (M7.2): w0 = IPv4 address. Resolves the address if needed,
/// sends an ICMP echo request, and replies with the round-trip time
/// in MICROSECONDS in word 1 — measured with `SYS_CLOCK_NOW`, so it
/// is the machine's own clock rather than a count of retries.
pub const ICMP_OP_PING: u64 = 3;
/// Receive-path counters (M7.2), packed into word 1: frames seen in
/// the low 16 bits, ARP in the next 16, IPv4 in the next, and frames
/// dropped by the demultiplexer in the top 16. A stack that cannot
/// say what it dropped is not demultiplexing, it is guessing.
pub const ICMP_OP_RXSTATS: u64 = 4;

// ---- UDP (M7.4, ADR-0033) ---------------------------------------------
//
// BIND returns an opaque HANDLE and possession of it is the authority
// to use that port. Not a port number the caller repeats back, and
// not an index: a random 64-bit token the service drew from rngd.
//
// This is the same idea as a kernel capability, implemented one layer
// down — authority by possession of an unforgeable reference, with
// deliberate passing of the token being delegation. It is NOT proof of
// identity: IPC v1 tells a server nothing about its caller, so the
// stack cannot say WHO holds a handle, only that whoever presents it
// holds it. (ADR-0033; the reasoning, and my earlier mistake about
// it, are in ADR-0032's appendix.)
//
//   BIND    w0 = port. Replies with the handle in word 1.
//   SEND    w0 = handle; inline = [dst ip(4) | dst port(2) | len(2) |
//           payload]. Replies with the bytes sent.
//   RECV    w0 = handle; inline = [timeout_us(8)]. Replies with the
//           datagram's FULL length in word 1 and
//           [src ip(4) | src port(2) | len(2) | payload…] inline —
//           first UDP_INLINE bytes. The rest is staged until drained.
//   RECV_CHUNK w0 = same bearer handle; inline = offset(2, big endian).
//           Replies with count in word 1 and up to MSG_BYTES bytes inline.
//           Offset must start at UDP_INLINE; last chunk releases the stage.
//   CLOSE   w0 = handle. Releases the binding.
pub const UDP_OP_BIND: u64 = 5;
pub const UDP_OP_SEND: u64 = 6;
pub const UDP_OP_RECV: u64 = 7;
pub const UDP_OP_CLOSE: u64 = 8;
pub const UDP_OP_RECV_CHUNK: u64 = 9;
/// DNS lookup (M7.5): w0 = length of dotted ASCII name in inline
/// request (1..=32); reply word 1 packs the IPv4 A address low byte
/// first, as ARP_OP_RESOLVE packs addresses. The stack owns the DNS
/// transaction, not the caller: no UDP handle is exposed by LOOKUP.
pub const DNS_OP_LOOKUP: u64 = 10;
pub const DNS_S_BAD_NAME: u64 = (-8i64) as u64;
pub const DNS_S_NO_ANSWER: u64 = (-9i64) as u64;
pub const DNS_S_BAD_REPLY: u64 = (-10i64) as u64;
pub const DNS_S_BUSY: u64 = (-11i64) as u64;

/// Bytes of datagram payload one request or reply message carries,
/// after the 8-byte address header above.
pub const UDP_INLINE: usize = MSG_BYTES - 8;

/// The handle presented does not name a live binding — either it was
/// never issued, or it has been closed. A forged one lands here too,
/// which is the point.
pub const UDP_S_BAD_HANDLE: u64 = (-5i64) as u64;
/// That port is already bound. A namespace rule, and a separate
/// question from authority.
pub const UDP_S_IN_USE: u64 = (-6i64) as u64;
/// Nothing arrived for this binding before the deadline.
pub const UDP_S_NO_DATA: u64 = (-7i64) as u64;

// ---- TCP active-open v1 (M7.6, ADR-0035) -------------------------------
// A single bounded outbound connection. Each operation requires the
// service-issued random bearer handle; no kernel TCP capability or pid.
// OPEN: w0 = dst IPv4 packed low-byte-first; inline[0..2] = dst port BE.
//       Sends SYN and immediately returns handle (ARP is bounded first).
// POLL: w0 = handle; inline[0..8] = max wait in microseconds. Advances
//       receive/retransmit once; word1 = state | (queued bytes << 8).
// WRITE: w0 = handle; inline[0] = length, [1..] = bytes (<=63).
// READ: w0 = handle; word1 = bytes returned, inline = up to 64 bytes.
// CLOSE: w0 = handle; initiates FIN after outstanding data is ACKed.
// RELEASE: w0 = handle; revokes it after CLOSED/FAILED.
pub const TCP_OP_OPEN: u64 = 11;
pub const TCP_OP_POLL: u64 = 12;
pub const TCP_OP_WRITE: u64 = 13;
pub const TCP_OP_READ: u64 = 14;
pub const TCP_OP_CLOSE: u64 = 15;
pub const TCP_OP_RELEASE: u64 = 16;
pub const TCP_CONNECTING: u64 = 1;
pub const TCP_ESTABLISHED: u64 = 2;
pub const TCP_CLOSING: u64 = 3;
pub const TCP_CLOSED: u64 = 4;
pub const TCP_FAILED: u64 = 5;
pub const TCP_S_BAD_HANDLE: u64 = (-12i64) as u64;
pub const TCP_S_BUSY: u64 = (-13i64) as u64;
pub const TCP_S_STATE: u64 = (-14i64) as u64;
pub const IP_PROTO_TCP: u8 = 6;

pub const IP_PROTO_UDP: u8 = 17;

pub const ARP_S_OK: u64 = 0;
pub const ARP_S_BAD_OP: u64 = (-1i64) as u64;
/// Nobody answered before the deadline, after every retry. The
/// address may exist and be silent; this is "no answer", not "no
/// such host".
pub const ARP_S_UNREACHABLE: u64 = (-2i64) as u64;
/// The echo request went out and nothing came back before the
/// deadline. Distinct from UNREACHABLE, which means the address could
/// not even be resolved: one is "no host", the other is "a host that
/// did not answer", and a diagnostic tool needs the difference.
pub const ICMP_S_NO_REPLY: u64 = (-4i64) as u64;

/// The driver below refused or is gone (see STATUS_SERVICE_GONE).
pub const ARP_S_LINK_DOWN: u64 = (-3i64) as u64;

/// The slirp network ArenaOS boots into: the guest address QEMU's
/// DHCP would hand out, and the gateway. Hardcoded until there is a
/// DHCP client — stated here as a FIXTURE fact rather than pretended
/// to be configuration.
pub const SLIRP_GUEST_IP: [u8; 4] = [10, 0, 2, 15];
pub const SLIRP_GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];

// ---- the fault-injection protocol (M6.5, ADR-0028) --------------------------
//
// The smallest service in the system, and the only one whose purpose
// is to be killed. It exists so that "a driver died while serving a
// request" is a case the OS has a TESTED answer for, rather than a
// situation nobody has ever run.
//
// Request words: w0 = unused, w1 = op.
//   PING      reply immediately (proves the service is alive and that
//             the endpoint works before and — after a restart — again).
//   HANG      take the request and never reply: park forever on a
//             notification nobody will ever signal. The suite destroys
//             the server while this request is in its hands.
//   SHUTDOWN  reply and exit cleanly.
// Reply word 0 is FAULT_S_OK.
pub const FAULT_OP_SHUTDOWN: u64 = 0;
pub const FAULT_OP_PING: u64 = 1;
pub const FAULT_OP_HANG: u64 = 2;

pub const FAULT_S_OK: u64 = 0;
pub const FAULT_S_BAD_OP: u64 = (-1i64) as u64;

/// The badge faultd sends on its notification just before parking in a
/// HANG, so the suite can inject the fault at a DETERMINISTIC moment
/// instead of guessing with a sleep.
pub const FAULT_BADGE_HANGING: u64 = 1 << 16;

/// The kernel's typed answer to a caller whose server was destroyed
/// (`STATUS_SERVICE_GONE`). Mirrored here because clients must be able
/// to name it: this is the status a well-written client RETRIES,
/// rather than the status it dies of.
pub const STATUS_SERVICE_GONE: i64 = -5;

// ---- timers (M7.0, ADR-0029) ------------------------------------------------
//
// `SYS_TIMER_ARM(notif_slot, badge, delay_us)` delivers `badge` on a
// notification the caller already holds, once `delay_us` of monotonic
// time has passed; it returns a timer id, and `SYS_TIMER_CANCEL(id)`
// disarms it. Nothing new to block on — a service already parks in
// SYS_WAIT on one notification with merged badge bits, so a timeout is
// just one more bit.
//
// A deadline means NOT BEFORE: timers are checked on the 100 Hz tick,
// so expect up to ~10 ms of lag and never assume finer. When a program
// needs to know how much time actually passed, it asks the clock.
//
// Badges are BITS (see CONSOLE_BADGE_GIVE_UP): a timeout that arrives
// in the same wake as real work must not hide it.

/// One tick of the kernel's timer, in microseconds — the granularity
/// below which a delay is meaningless.
pub const TICK_US: u64 = 10_000;

// ---- diagnostic exit codes shared by both binaries ---------------------------

pub const EXIT_OK: u64 = 42;
/// fstest's SECOND verified-success code (M5.4): the volume already
/// held the test file at mount — it was committed by an EARLIER boot
/// and survived. The persisted branch verifies it byte-for-byte with
/// no write at all, so its device-operation contract differs from the
/// fresh-volume one; the code tells the m5 suite which contract held.
pub const EXIT_OK_PERSISTED: u64 = 43;
pub const EXIT_WRITE_REFUSED: u64 = 97;
pub const EXIT_PANIC: u64 = 99;

// ---- syscall stubs (ABI v1; see the module header) ---------------------------

/// A call with no arguments (M7.0: `SYS_CLOCK_NOW` is the first one —
/// asking what time it is needs nothing but the question).
///
/// # Safety
/// As the other stubs: ABI v1 register contract, kernel stack.
pub unsafe fn syscall0(nr: u64) -> i64 {
    let ret: i64;
    // SAFETY: caller contract — ABI v1 register arguments; the clobber
    // list is the documented caller-saved set; `nostack` (the stub runs
    // on the kernel stack).
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            lateout("rdi") _,
            lateout("rsi") _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

pub unsafe fn syscall1(nr: u64, a0: u64) -> i64 {
    let ret: i64;
    // SAFETY: caller contract — ABI v1 register arguments; the clobber
    // list is the documented caller-saved set; `nostack` (the stub runs
    // on the kernel stack).
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            lateout("rsi") _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

pub unsafe fn syscall2(nr: u64, a0: u64, a1: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

pub unsafe fn syscall3(nr: u64, a0: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

pub unsafe fn syscall4(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1 (a3 rides R10 per ABI v1).
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
            inlateout("r10") a3 => _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

pub unsafe fn syscall5(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1 (a3/a4 ride R10/R8 per ABI v1).
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
            inlateout("r10") a3 => _,
            inlateout("r8") a4 => _,
            lateout("r9") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

pub unsafe fn syscall6(nr: u64, a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> i64 {
    let ret: i64;
    // SAFETY: as syscall1 (a5 rides R9 per ABI v1). IPC v1.1 calls
    // MUST use this stub (or explicitly zero R9): the kernel validates
    // a stale r9 as an inline-message pointer.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            inlateout("rdi") a0 => _,
            inlateout("rsi") a1 => _,
            inlateout("rdx") a2 => _,
            inlateout("r10") a3 => _,
            inlateout("r8") a4 => _,
            inlateout("r9") a5 => _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

// ---- console output (the shell's discipline: chunked, honest exits) ----------

/// Dispatcher's largest accepted debug_write (kernel-side WRITE_MAX).
pub const WRITE_MAX: usize = 256;

static mut OUT: [u8; WRITE_MAX] = [0; WRITE_MAX];

/// Write `buf` to the console, chunked to the dispatcher's bound. A
/// refusal has no recovery — the console is this image's only voice, so
/// the honest outcome is a diagnostic exit.
pub fn write_all(mut buf: &[u8]) {
    while !buf.is_empty() {
        let n = if buf.len() > WRITE_MAX {
            WRITE_MAX
        } else {
            buf.len()
        };
        // SAFETY: `buf` is always image-owned memory inside the
        // registered user regions (rodata in the text segment, or the
        // OUT/.bss static); wrapper contract as above.
        let w = unsafe { syscall2(SYS_DEBUG_WRITE, buf.as_ptr() as u64, n as u64) };
        if w != n as i64 {
            // SAFETY: thread_exit diverges; the code is the contract.
            unsafe { syscall2(SYS_THREAD_EXIT, EXIT_WRITE_REFUSED, 0) };
        }
        buf = &buf[n..];
    }
}

pub fn write_str(s: &str) {
    write_all(s.as_bytes());
}

/// The composed-line writer: bytes accumulate in OUT, `flush` sends
/// them. Both binaries are single-threaded; OUT is touched only here.
pub struct Out {
    pub n: usize,
}

impl Out {
    pub fn new() -> Out {
        Out { n: 0 }
    }
    pub fn push(&mut self, b: u8) {
        if self.n < WRITE_MAX {
            // SAFETY: OUT is this image's own .bss static, written
            // single-threaded at a bounds-checked offset.
            unsafe { *core::ptr::addr_of_mut!(OUT).cast::<u8>().add(self.n) = b };
            self.n += 1;
        }
    }
    pub fn str(&mut self, s: &str) {
        for b in s.bytes() {
            self.push(b);
        }
    }
    pub fn bytes(&mut self, s: &[u8]) {
        for &b in s {
            self.push(b);
        }
    }
    pub fn u64(&mut self, v: u64) {
        let mut tmp = [0u8; 20];
        let mut n = 0usize;
        let mut x = v;
        loop {
            tmp[n] = b'0' + (x % 10) as u8;
            n += 1;
            x /= 10;
            if x == 0 {
                break;
            }
        }
        while n > 0 {
            n -= 1;
            self.push(tmp[n]);
        }
    }
    pub fn i64(&mut self, v: i64) {
        if v < 0 {
            self.push(b'-');
            self.u64(v.wrapping_neg() as u64);
        } else {
            self.u64(v as u64);
        }
    }
    pub fn hex(&mut self, v: u64) {
        self.str("0x");
        for i in (0..16).rev() {
            let d = ((v >> (i * 4)) & 0xF) as u8;
            self.push(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        }
    }
    /// Two ASCII hex digits for one byte (zero-padded, lowercase) —
    /// compact byte dumps (MAC addresses) that stay inside WRITE_MAX
    /// where `hex`'s full 64-bit form would overflow the line.
    pub fn hex2(&mut self, b: u8) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.push(HEX[(b >> 4) as usize]);
        self.push(HEX[(b & 0xF) as usize]);
    }
    /// CRLF — debug_write copies raw bytes (no \n translation), and the
    /// console is a terminal.
    pub fn crlf(&mut self) {
        self.push(b'\r');
        self.push(b'\n');
    }
    pub fn flush(&mut self) {
        // SAFETY: OUT[..n] are bytes this Out wrote; single-threaded.
        let buf =
            unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(OUT).cast::<u8>(), self.n) };
        write_all(buf);
        self.n = 0;
    }
}

/// Log one composed line immediately (the driver's per-event voice).
pub fn log_line(f: impl FnOnce(&mut Out)) {
    let mut o = Out::new();
    f(&mut o);
    o.crlf();
    o.flush();
}

// ---- volatile memory accessors (MMIO registers + virtqueue rings) ------------
//
// Every device/ring touch goes through these: the compiler must never
// elide, merge, or invent accesses to memory a device reads and writes
// concurrently. Ordering to the DEVICE is x86-TSO's job plus the
// explicit fences the driver places around ring-index updates.

pub unsafe fn r8(va: u64) -> u8 {
    // SAFETY: caller contract — `va` is a mapped, suitably aligned
    // register/ring byte this image owns or was granted.
    unsafe { core::ptr::read_volatile(va as *const u8) }
}
pub unsafe fn w8(va: u64, v: u8) {
    // SAFETY: as r8.
    unsafe { core::ptr::write_volatile(va as *mut u8, v) }
}
pub unsafe fn r16(va: u64) -> u16 {
    // SAFETY: as r8; 2-aligned.
    unsafe { core::ptr::read_volatile(va as *const u16) }
}
pub unsafe fn w16(va: u64, v: u16) {
    // SAFETY: as r16.
    unsafe { core::ptr::write_volatile(va as *mut u16, v) }
}
pub unsafe fn r32(va: u64) -> u32 {
    // SAFETY: as r8; 4-aligned.
    unsafe { core::ptr::read_volatile(va as *const u32) }
}
pub unsafe fn w32(va: u64, v: u32) {
    // SAFETY: as r32.
    unsafe { core::ptr::write_volatile(va as *mut u32, v) }
}

/// A full-store fence: ring writes (descriptors, avail index) must be
/// globally visible before the doorbell write reaches the device.
pub fn store_fence() {
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    // SAFETY: SFENCE is unprivileged and side-effect-free beyond
    // ordering this CPU's stores.
    unsafe { core::arch::asm!("sfence", options(nostack, preserves_flags)) };
}
