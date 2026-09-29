//! ArenaOS console channel service — `consoled`, the userspace
//! virtio-console driver (M6.4, ADR-0027). Spawn-registry image 12,
//! spawned at boot by the kernel (and as a short-lived test instance
//! by the m6 suite) with kernel-literal grants:
//!
//! - slot 0: `Mmio` over the BAR carrying the virtio structures
//!   (READ|WRITE — the handshake writes registers),
//! - slot 1: `Endpoint` (READ — the serve side, service mode only),
//! - slot 2: `Notification` (READ|WRITE — the interrupt relay target,
//!   and the console-output wake, and the spawner's give-up word:
//!   three sources, one place to block),
//! - slot 3: `ConsoleInput` (WRITE — PRODUCTION ONLY),
//! - slot 4: `ConsoleOutput` (READ — PRODUCTION ONLY).
//!
//! Everything else is this program's own work, all from ring 3 on the
//! SHARED virtio core (`userspace/virtio.rs` — the fifth driver on it,
//! and the first with a queue in EACH direction): discover the
//! resolved device record, self-map the window, run the §3.1
//! handshake, build two split virtqueues (one packed frame each, the
//! receive buffers sharing the receive queue's frame), stock the
//! receive side, arm two MSI-X relays into one notification, then run
//! in one of two modes.
//!
//! **A port is not a console.** The device moves bytes; what makes a
//! console is the kernel's line discipline on the way in and the
//! kernel's output on the way out. So in production this driver holds
//! BOTH console capabilities and becomes a second channel onto the
//! machine's one console:
//!
//! - host → guest: bytes off the receive queue go into the kernel's
//!   line discipline through `SYS_CONSOLE_PUSH` — the identical entry
//!   point the UART RX ISR and `inputd` use (ADR-0026). Typing into
//!   `nc -U` drives the same `arena>` prompt.
//! - guest → host: `SYS_CONSOLE_ATTACH` turns on the kernel's console
//!   output mirror and names this notification; every wake pulls with
//!   `SYS_CONSOLE_PULL` and sends the bytes out the transmit queue.
//!
//! Neither direction displaces the serial port, which stays the
//! kernel's own for logs and panics.
//!
//! **Mode selection is a capability probe, not a spawn argument** (the
//! ADR-0026 pattern): a zero-length `SYS_CONSOLE_PUSH` and a
//! `SYS_CONSOLE_ATTACH` decide it. The m6 suite withholds both, so its
//! instance serves the port over IPC instead — the same driver and the
//! same queues, with an observable boundary.
//!
//! Exit codes (the diagnostic contract with the m6 suite): 42 clean
//! shutdown (the poison request was served), 80..89 stage failures,
//! 99 panic.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[path = "../../abi.rs"]
mod abi;
use abi::*;

#[path = "../../virtio.rs"]
mod virtio;
use virtio::*;

// ---- the grant layout (kernel-literal: entry.rs / m6.rs) ---------------------

const SLOT_MMIO: u64 = 0;
const SLOT_EP: u64 = 1;
const SLOT_DIAG: u64 = 5; // ADR-0047 boot-granted proof
const SLOT_NOTIF: u64 = 2;
/// The console-input authority — PRODUCTION ONLY.
const SLOT_CON_IN: u64 = 3;
/// The console-output authority — PRODUCTION ONLY.
const SLOT_CON_OUT: u64 = 4;

/// Owned frame slots: one frame per queue. The receive frame also
/// carries the receive buffers above its ring areas (asserted, not
/// assumed); the transmit frame carries the send buffers the same way.
const SLOT_FRAME_BASE: u64 = 8;
const FRAMES_TOTAL: usize = 2;
const FRAME_RX: usize = 0;
const FRAME_TX: usize = 1;

/// Badges merged into the ONE notification this driver blocks on
/// (netd's two-vector pattern, ADR-0024, extended with a wake that is
/// not an interrupt at all).
///
/// DISJOINT BITS, tested with `&` — `ipc::notify` ORs badges together,
/// so two sources that can arrive in one wake must not share a bit.
/// See the note on `CONSOLE_BADGE_GIVE_UP` in abi.rs for what numeric
/// words cost here.
const BADGE_RX: u64 = 1 << 16;
const BADGE_TX: u64 = 1 << 17;
/// The kernel's console output mirror has bytes (M6.4: not a device
/// interrupt — the kernel notifies this same notification, which is
/// exactly why the driver has only one thing to block on).
const BADGE_CON_OUT: u64 = 1 << 18;

// ---- the diagnostic exit contract (m6.rs maps every code) --------------------

const EXIT_DEV_INFO: u64 = 80;
const EXIT_MAP: u64 = 81;
const EXIT_HANDSHAKE: u64 = 82;
const EXIT_QUEUE: u64 = 83;
const EXIT_RELAY: u64 = 84;
const EXIT_RECV: u64 = 85;
const EXIT_WAIT: u64 = 86;
const EXIT_SEND: u64 = 87;
const EXIT_REPLY: u64 = 88;
const EXIT_PUSH: u64 = 89;

// ---- virtio-console specifics (the shared 1.0 core is userspace/virtio.rs) ---

/// virtio-console's PCI ids: transitional 0x1003 (what
/// `virtio-serial-pci` presents on q35 today — the type arrives in the
/// subsystem id, which the kernel's scan already resolves) and modern
/// 0x1040 + 3 = 0x1043.
const DEV_ID_CONSOLE_TRANS: u64 = 0x1003;
const DEV_ID_CONSOLE_MODERN: u64 = 0x1043;

/// Port 0's queues. The virtio-console layout puts port 0 first:
/// queue 0 receive (device → driver), queue 1 transmit (driver →
/// device). Queues 2/3 are the control pair and exist only under
/// `VIRTIO_CONSOLE_F_MULTIPORT`, which this driver deliberately does
/// NOT negotiate — see the feature note below.
const QUEUE_RX: u16 = 0;
const QUEUE_TX: u16 = 1;

/// The queue-size cap (the driver's frame budget: ring areas AND
/// buffers share one page per queue). The device's own size wins when
/// it is smaller.
const QUEUE_MAX: u16 = 32;

/// Feature bits this driver understands. It negotiates NONE of them —
/// `FEATURE_VERSION_1` only:
///
/// - `SIZE` (bit 0) would add a console-dimensions config field this
///   driver has no use for (there is no terminal to resize),
/// - `MULTIPORT` (bit 1) would add the control queues, a port
///   discovery protocol, and per-port open/close events. Declining it
///   is not laziness: QEMU's `set_status` marks port 0
///   `guest_connected` as soon as a NON-multiport guest reaches
///   DRIVER_OK (hw/char/virtio-serial-bus.c), so one port works in
///   both directions with two queues and no control protocol at all.
///   The fixture pins `max_ports=1`, at which QEMU does not even offer
///   the bit,
/// - `EMERG_WRITE` (bit 2) is a config-space escape hatch for panics
///   before the queues exist. ArenaOS panics on the serial port, which
///   is the kernel's own and always up; a second panic path through a
///   ring-3 driver would be a worse one.
const CONSOLE_FEATURES_ACCEPTED: u32 = 0;

/// One receive buffer. The host writes arbitrary byte runs; 64 keeps
/// a screenful of paste inside a handful of descriptors.
const RX_BUF_BYTES: u64 = 64;
/// How many receive buffers stay posted. The device writes only what
/// it has, so this is a burst bound, not a rate.
const RX_BUFS: u16 = 16;
/// Where the receive buffers live inside the receive frame (above the
/// ring areas; the setup asserts they fit).
const RX_BUF_OFF: u64 = 1024;

/// One transmit buffer, sized to the kernel's pull bound so a full
/// pull always fits one descriptor.
const TX_BUF_BYTES: u64 = CONSOLE_PULL_MAX;
/// Transmit buffers in flight. More than one because the driver must
/// never block waiting for the device while the receive side has work
/// — completions arrive as interrupts on the same notification.
const TX_BUFS: u16 = 8;
/// Where the transmit buffers live inside the transmit frame.
const TX_BUF_OFF: u64 = 1024;

/// Bytes buffered from the host between consumers (service mode's
/// clients arrive whenever they like; the port does not wait).
const RX_RING: usize = 256;

/// Guard against a device that interrupts without publishing anything.
const HARVEST_REWAITS: u32 = 64;

// ---- the driver ---------------------------------------------------------

struct Drv {
    rx: Queue,
    tx: Queue,
    rx_buf_va: u64,
    rx_buf_phys: u64,
    tx_buf_va: u64,
    tx_buf_phys: u64,
    /// Which transmit descriptors the device still owns.
    tx_busy: [bool; TX_BUFS as usize],
    /// Received bytes waiting for a consumer.
    ring: [u8; RX_RING],
    head: usize,
    len: usize,
    // --- counters (the suite's evidence) ---
    rx_batches: u64,
    tx_batches: u64,
    bytes_in: u64,
    bytes_out: u64,
    dropped: u64,
    give_ups: u64,
    /// STICKY: the spawner has called the wait off. Sticky because the
    /// badge can arrive while this driver is blocked for a different
    /// reason (a WRITE waiting for a transmit buffer), and a
    /// supervisor's "stop waiting" must not be consumed and forgotten
    /// by whichever wait happened to be running.
    gave_up: bool,
}

impl Drv {
    /// Post receive descriptor `id` back to the device.
    ///
    /// # Safety
    /// The ring VAs are this image's own mapped frames; the buffer
    /// span belongs to this driver; single-threaded.
    unsafe fn post_rx(&mut self, id: u16) {
        let off = u64::from(id) * RX_BUF_BYTES;
        // SAFETY: method contract.
        unsafe {
            virtio::desc_write(
                self.rx.desc_va,
                id,
                self.rx_buf_phys + off,
                RX_BUF_BYTES as u32,
                DESC_F_WRITE,
                0,
            );
            self.rx.publish(id);
        }
    }

    /// Append one received byte to the ring (oldest-drop, counted: a
    /// host that pastes faster than anyone reads must not wedge the
    /// device side).
    fn push(&mut self, b: u8) {
        if self.len == RX_RING {
            self.head = (self.head + 1) % RX_RING;
            self.len -= 1;
            self.dropped += 1;
        }
        let slot = (self.head + self.len) % RX_RING;
        self.ring[slot] = b;
        self.len += 1;
        self.bytes_in += 1;
    }

    /// Take up to `out.len()` buffered bytes.
    fn take(&mut self, out: &mut [u8]) -> usize {
        let n = core::cmp::min(out.len(), self.len);
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o = self.ring[(self.head + i) % RX_RING];
        }
        self.head = (self.head + n) % RX_RING;
        self.len -= n;
        n
    }

    /// Drain the receive used ring to its end, buffering every byte
    /// the device wrote and re-posting each descriptor. Returns the
    /// number of used entries consumed.
    ///
    /// # Safety
    /// Method contract as `post_rx`.
    unsafe fn harvest_rx(&mut self) -> u32 {
        let mut n = 0;
        // SAFETY: method contract.
        unsafe {
            while self.rx.used_seen != self.rx.used_idx() {
                let slot = self.rx.used_seen % self.rx.qsz;
                let (id, len) = self.rx.used_entry(slot);
                self.rx.used_seen = self.rx.used_seen.wrapping_add(1);
                if id < u32::from(RX_BUFS) {
                    let base = self.rx_buf_va + u64::from(id) * RX_BUF_BYTES;
                    let take = core::cmp::min(u64::from(len), RX_BUF_BYTES);
                    for i in 0..take {
                        self.push(r8(base + i));
                    }
                    self.post_rx(id as u16);
                }
                n += 1;
            }
        }
        n
    }

    /// Reclaim completed transmit descriptors.
    ///
    /// # Safety
    /// Method contract as `post_rx`.
    unsafe fn harvest_tx(&mut self) -> u32 {
        let mut n = 0;
        // SAFETY: method contract.
        unsafe {
            while self.tx.used_seen != self.tx.used_idx() {
                let slot = self.tx.used_seen % self.tx.qsz;
                let (id, _) = self.tx.used_entry(slot);
                self.tx.used_seen = self.tx.used_seen.wrapping_add(1);
                if let Some(busy) = self.tx_busy.get_mut(id as usize) {
                    *busy = false;
                }
                n += 1;
            }
        }
        n
    }

    /// Send `bytes` out the port. Returns false when every transmit
    /// buffer is still with the device — the caller keeps the data and
    /// retries after the next completion, rather than blocking a
    /// driver that also has a receive side to service.
    ///
    /// # Safety
    /// Method contract as `post_rx`.
    unsafe fn send(&mut self, bytes: &[u8]) -> bool {
        let Some(id) = (0..TX_BUFS).find(|i| !self.tx_busy[*i as usize]) else {
            return false;
        };
        let n = core::cmp::min(bytes.len() as u64, TX_BUF_BYTES);
        let off = u64::from(id) * TX_BUF_BYTES;
        // SAFETY: method contract; the span is this driver's own frame.
        unsafe {
            for (i, b) in bytes[..n as usize].iter().enumerate() {
                w8(self.tx_buf_va + off + i as u64, *b);
            }
            virtio::desc_write(
                self.tx.desc_va,
                id,
                self.tx_buf_phys + off,
                n as u32,
                0, // driver-readable: the device only reads a transmit buffer
                0,
            );
            self.tx_busy[id as usize] = true;
            self.tx.publish(id);
        }
        self.bytes_out += n;
        true
    }

    /// Block on the notification and service whatever woke us.
    /// Returns the badge word so the caller can tell a give-up from
    /// ordinary traffic. Every wake harvests BOTH queues: the badges
    /// say what happened, but the used rings are the truth.
    ///
    /// # Safety
    /// Method contract as `post_rx`.
    unsafe fn wait_once(&mut self) -> Result<u64, ()> {
        // SAFETY: wrapper contract.
        let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
        if b < 0 {
            log_line(|o| {
                o.str("consoled: wait returned ");
                o.i64(b);
            });
            return Err(());
        }
        let badge = b as u64;
        if badge & CONSOLE_BADGE_GIVE_UP != 0 {
            self.gave_up = true;
            self.give_ups += 1;
        }
        if badge & BADGE_RX != 0 {
            // SAFETY: method contract.
            let got = unsafe { self.harvest_rx() };
            if got > 0 {
                self.rx_batches += 1;
            }
        }
        if badge & BADGE_TX != 0 {
            // SAFETY: method contract.
            let got = unsafe { self.harvest_tx() };
            if got > 0 {
                self.tx_batches += 1;
            }
        }
        Ok(badge)
    }

    /// Block until at least one byte has arrived from the host.
    /// `Ok(false)` = the spawner called the wait off (ADR-0026's
    /// give-up pattern: a port with nothing on its far end is
    /// indistinguishable from one whose far end is merely slow, so the
    /// decision to stop waiting belongs to the supervisor).
    ///
    /// # Safety
    /// Method contract as `post_rx`.
    unsafe fn wait_for_data(&mut self) -> Result<bool, ()> {
        let mut idle = 0u32;
        while self.len == 0 {
            if self.gave_up {
                return Ok(false);
            }
            // SAFETY: method contract.
            let _badge = unsafe { self.wait_once()? };
            if self.len == 0 {
                idle += 1;
                if idle > HARVEST_REWAITS {
                    log("consoled: wakes without received bytes");
                    return Err(());
                }
            }
        }
        Ok(true)
    }
}

/// The driver entry: the spawn protocol's first thread lands here at
/// ring 3 with the grant slots filled.
///
/// # Safety
/// Entered exactly as inputd's `_start`: ring 3, RSP at the derived
/// stack top, the address space the kernel loaded and validated,
/// grants in slots 0..2 (and 3/4 in production).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and its granted/allocated windows, inside
    // the address space the kernel loaded and validated.
    // Single-threaded; every device and ring touch is volatile.
    unsafe {
        log("consoled: starting — ArenaOS console channel service (M6.4, ADR-0027)");

        // 1. Discovery, window map, handshake — the shared core. Two
        //    MSI-X entries are required (one per queue) and no feature
        //    bits are accepted beyond VERSION_1.
        let info = virtio::discover(
            "consoled",
            "virtio-console",
            DEV_ID_CONSOLE_TRANS,
            DEV_ID_CONSOLE_MODERN,
            2,
        )
        .unwrap_or_else(|e| vfail(e));
        let w = virtio::map_window("consoled", "virtio-console", &info, SLOT_MMIO)
            .unwrap_or_else(|e| vfail(e));
        virtio::handshake(
            "consoled",
            &w,
            CONSOLE_FEATURES_ACCEPTED,
            FEATURE_VERSION_1,
            2,
        )
        .unwrap_or_else(|e| vfail(e));

        // 2. Two owned frames: one per queue, each carrying its own
        //    buffers above its ring areas.
        let mut phys = [0u64; FRAMES_TOTAL];
        let mut va = [0u64; FRAMES_TOTAL];
        virtio::alloc_frames(
            "consoled",
            SLOT_FRAME_BASE,
            FRAMES_TOTAL,
            &mut phys,
            &mut va,
        )
        .unwrap_or_else(|e| vfail(e));

        // 3. The two queues, each with its own MSI-X entry relayed
        //    into the SAME notification under a distinct badge —
        //    netd's merged-badge shape (ADR-0024), which is what lets
        //    this driver have exactly one place to block.
        let (rx, rx_vec) = virtio::queue_setup(
            "consoled",
            &w,
            &info,
            QUEUE_RX,
            QUEUE_MAX,
            RingMem::Packed {
                frame_phys: phys[FRAME_RX],
                frame_va: va[FRAME_RX],
            },
            IrqPlan {
                msix_entry: 0,
                slot_notif: SLOT_NOTIF,
                badge: BADGE_RX,
            },
        )
        .unwrap_or_else(|e| vfail(e));
        let (tx, tx_vec) = virtio::queue_setup(
            "consoled",
            &w,
            &info,
            QUEUE_TX,
            QUEUE_MAX,
            RingMem::Packed {
                frame_phys: phys[FRAME_TX],
                frame_va: va[FRAME_TX],
            },
            IrqPlan {
                msix_entry: 1,
                slot_notif: SLOT_NOTIF,
                badge: BADGE_TX,
            },
        )
        .unwrap_or_else(|e| vfail(e));

        // The buffers share their queue's frame — PROVE they fit.
        for (qsz, off, count, bytes) in [
            (rx.qsz, RX_BUF_OFF, u64::from(RX_BUFS), RX_BUF_BYTES),
            (tx.qsz, TX_BUF_OFF, u64::from(TX_BUFS), TX_BUF_BYTES),
        ] {
            let (_, _, used_off) = virtio::ring_offsets(qsz);
            let rings_end = used_off + 6 + 8 * u64::from(qsz);
            if rings_end > off || off + count * bytes > 4096 || count > u64::from(qsz) {
                fail(
                    EXIT_QUEUE,
                    "the port buffers do not fit their ring frame beside the rings",
                );
            }
        }

        let mut drv = Drv {
            rx,
            tx,
            rx_buf_va: va[FRAME_RX] + RX_BUF_OFF,
            rx_buf_phys: phys[FRAME_RX] + RX_BUF_OFF,
            tx_buf_va: va[FRAME_TX] + TX_BUF_OFF,
            tx_buf_phys: phys[FRAME_TX] + TX_BUF_OFF,
            tx_busy: [false; TX_BUFS as usize],
            ring: [0u8; RX_RING],
            head: 0,
            len: 0,
            rx_batches: 0,
            tx_batches: 0,
            bytes_in: 0,
            bytes_out: 0,
            dropped: 0,
            give_ups: 0,
            gave_up: false,
        };

        // 4. DRIVER_OK first, THEN stock the receive side — the
        //    opposite of inputd's order, and for a device-specific
        //    reason worth stating (ADR-0027, and QEMU's
        //    hw/char/virtio-serial-bus.c):
        //
        //    QEMU asks `chr_can_read` before handing the port any host
        //    bytes, and that answers 0 while the guest is not yet
        //    DRIVER_OK. A chardev whose frontend says "cannot read"
        //    is PAUSED, and the only thing that resumes it is
        //    `handle_input` → `guest_writable` → `accept_input`, which
        //    fires when the guest adds buffers — but only if the port
        //    is already `guest_connected`, which for a non-multiport
        //    guest happens exactly at DRIVER_OK.
        //
        //    So buffers posted BEFORE the status bit leave the host
        //    side paused forever: the first thing anyone types on the
        //    socket goes nowhere. Publishing after DRIVER_OK rings the
        //    doorbell with the port connected, which resumes it. The
        //    window in between is empty of risk — the device has
        //    nothing to deliver until a human or a harness sends
        //    something.
        virtio::driver_ok(&w);
        for id in 0..RX_BUFS {
            drv.post_rx(id);
        }
        log_line(|o| {
            o.str("consoled: virtio-console ready — DRIVER_OK, rx/tx size ");
            o.u64(u64::from(drv.rx.qsz));
            o.str("/");
            o.u64(u64::from(drv.tx.qsz));
            o.str(", ");
            o.u64(u64::from(RX_BUFS));
            o.str(" rx buffers posted; relay vectors ");
            o.i64(rx_vec);
            o.str("/");
            o.i64(tx_vec);
        });

        // 5. Which service is this? Two capability probes, in the
        //    order that matters: the OUTPUT attach is the one with a
        //    side effect (it turns the kernel's mirror on), so it goes
        //    second — an instance that can push but not attach is a
        //    misconfiguration, not a mode, and says so.
        let can_push = syscall3(SYS_CONSOLE_PUSH, SLOT_CON_IN, 0, 0) >= 0;
        if can_push {
            let r = syscall3(SYS_CONSOLE_ATTACH, SLOT_CON_OUT, SLOT_NOTIF, BADGE_CON_OUT);
            if r < 0 {
                log_line(|o| {
                    o.str("consoled: console input authority without output: attach returned ");
                    o.i64(r);
                });
                fail(EXIT_PUSH, "the console grants are incomplete");
            }
            console_loop(&mut drv);
        }
        service_loop(&mut drv);
    }
}

/// PRODUCTION: be a second console channel. Never returns.
///
/// # Safety
/// As `_start`, and slots 3/4 have been PROVEN to carry the console
/// authorities by the probes above.
unsafe fn console_loop(drv: &mut Drv) -> ! {
    log("consoled: console mode — the port is a second console (serial stays the kernel's own)");
    // MUTABLE on purpose: the kernel WRITES into this buffer through
    // the pointer below. An immutable array is a promise to the
    // compiler that nothing changes it — and it may live in read-only
    // memory, which this image's own W^X mapping enforces.
    let mut pull = [0u8; CONSOLE_PULL_MAX as usize];
    let mut push = [0u8; CONSOLE_PUSH_MAX as usize];
    // Bytes pulled from the kernel that the device could not take yet
    // (every transmit buffer in flight). Held here until a completion
    // frees one — never dropped, never re-pulled.
    let mut held = 0usize;
    loop {
        // SAFETY: function contract; the wait blocks on the merged
        // notification and the harvests read this driver's own frames.
        unsafe {
            // Host → guest: whatever the port received goes into the
            // kernel's line discipline, in CONSOLE_PUSH_MAX runs.
            loop {
                let n = drv.take(&mut push);
                if n == 0 {
                    break;
                }
                let r = syscall3(
                    SYS_CONSOLE_PUSH,
                    SLOT_CON_IN,
                    push.as_ptr() as u64,
                    n as u64,
                );
                if r != n as i64 {
                    log_line(|o| {
                        o.str("consoled: console push of ");
                        o.u64(n as u64);
                        o.str(" byte(s) returned ");
                        o.i64(r);
                    });
                    fail(EXIT_PUSH, "SYS_CONSOLE_PUSH refused");
                }
            }

            // Guest → host: drain the kernel's output mirror into the
            // port for as long as both have room.
            loop {
                if held == 0 {
                    let got = syscall3(
                        SYS_CONSOLE_PULL,
                        SLOT_CON_OUT,
                        pull.as_mut_ptr() as u64,
                        CONSOLE_PULL_MAX,
                    );
                    if got < 0 {
                        log_line(|o| {
                            o.str("consoled: console pull returned ");
                            o.i64(got);
                        });
                        fail(EXIT_SEND, "SYS_CONSOLE_PULL refused");
                    }
                    held = got as usize;
                    if held == 0 {
                        break; // mirror empty: the notification re-arms
                    }
                }
                if !drv.send(&pull[..held]) {
                    break; // every transmit buffer in flight; a completion wakes us
                }
                held = 0;
            }

            if drv.wait_once().is_err() {
                fail(EXIT_WAIT, "the notification wait failed");
            }
        }
    }
}

/// THE SUITE'S INSTANCE: serve the port over the endpoint. Ends
/// through the poison request's clean exit.
///
/// # Safety
/// As `_start`.
unsafe fn service_loop(drv: &mut Drv) -> ! {
    log("consoled: service mode — no console authority, serving the port on the endpoint");
    // IPC v1.1 (ADR-0018): the request's two words plus any landed cap
    // arrive in `w`; the INLINE message (a WRITE's bytes) arrives in
    // `inbox`, which the same syscall fills.
    let mut w = [0u64; 3];
    let mut inbox = [0u8; MSG_BYTES];
    let mut out = [0u8; MSG_BYTES];
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
                    o.str("consoled: IPC_RECV returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "the serve-side receive failed");
            }
            let (n_or_want, op, landed) = (w[0], w[1], w[2]);
            if landed != CAP_NONE && op != CONSOLE_OP_SHUTDOWN {
                // This service lends nothing and takes nothing.
                let _ = syscall1(SYS_CAP_DESTROY, landed);
            }
            match op {
                CONSOLE_OP_SHUTDOWN => {
                    if !take_diagnostic(landed, SLOT_DIAG) {
                        reply(CONSOLE_S_BAD_OP, 0);
                        continue;
                    }
                    log_line(|o| {
                        o.str("consoled: poison shutdown — ");
                        o.u64(drv.bytes_out);
                        o.str(" byte(s) sent, ");
                        o.u64(drv.bytes_in);
                        o.str(" received over ");
                        o.u64(drv.rx_batches);
                        o.str(" rx batch(es), ");
                        o.u64(drv.dropped);
                        o.str(" dropped");
                    });
                    reply(CONSOLE_S_OK, drv.bytes_out);
                    syscall1(SYS_THREAD_EXIT, EXIT_OK);
                }
                CONSOLE_OP_WRITE => {
                    let n = n_or_want;
                    if n == 0 || n > CONSOLE_MSG_MAX {
                        reply(CONSOLE_S_BAD_LEN, 0);
                        continue;
                    }
                    // Retry until a transmit buffer frees: a WRITE is
                    // synchronous to its caller by contract, and the
                    // only thing that can free one is a completion.
                    while !drv.send(&inbox[..n as usize]) {
                        if drv.wait_once().is_err() {
                            fail(EXIT_WAIT, "the notification wait failed");
                        }
                    }
                    reply(CONSOLE_S_OK, n);
                }
                CONSOLE_OP_READ => {
                    let want = n_or_want;
                    if want == 0 || want > CONSOLE_MSG_MAX {
                        reply(CONSOLE_S_BAD_LEN, 0);
                        continue;
                    }
                    match drv.wait_for_data() {
                        Ok(true) => {}
                        Ok(false) => {
                            log("consoled: the wait was called off — nothing arrived on the port");
                            reply(CONSOLE_S_NO_DATA, 0);
                            continue;
                        }
                        Err(()) => fail(EXIT_WAIT, "the notification wait failed"),
                    }
                    let n = drv.take(&mut out[..want as usize]);
                    let r = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        CONSOLE_S_OK,
                        n as u64,
                        CAP_NONE,
                        out.as_ptr() as u64,
                    );
                    if r < 0 {
                        fail(EXIT_REPLY, "the READ reply was refused");
                    }
                }
                _ => reply(CONSOLE_S_BAD_OP, 0),
            }
        }
    }
}

/// Reply with status and one word, no inline payload.
///
/// # Safety
/// A reply is legal exactly once per received call; the caller is
/// inside the serve loop that received one.
unsafe fn reply(status: u64, w1: u64) {
    // SAFETY: wrapper contract.
    let r = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, status, w1, CAP_NONE, 0) };
    if r < 0 {
        fail(EXIT_REPLY, "the reply was refused");
    }
}

// ---- diagnostics -------------------------------------------------------------

fn log(s: &str) {
    log_line(|o| o.str(s));
}

/// Map a shared-core stage error onto THIS driver's exit contract.
fn vfail(e: VErr) -> ! {
    let (code, what) = match e {
        VErr::DevInfo(w) => (EXIT_DEV_INFO, w),
        VErr::Map(w) => (EXIT_MAP, w),
        VErr::Handshake(w) => (EXIT_HANDSHAKE, w),
        VErr::Queue(w) => (EXIT_QUEUE, w),
        VErr::Relay(w) => (EXIT_RELAY, w),
    };
    fail(code, what)
}

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("consoled: FAIL: ");
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

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    fail(EXIT_PANIC, "panic")
}
