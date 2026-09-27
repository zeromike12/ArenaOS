//! ArenaOS input service — `inputd`, the userspace virtio-input
//! keyboard driver (M6.3, ADR-0026). Spawn-registry image 10, spawned
//! at boot by the kernel (and as a short-lived test instance by the m6
//! suite) with kernel-literal grants:
//!
//! - slot 0: `Mmio` over the BAR carrying the virtio structures
//!   (READ|WRITE — the handshake writes registers),
//! - slot 1: `Endpoint` (READ — the serve side of the input service),
//! - slot 2: `Notification` (READ|WRITE — the interrupt relay target),
//! - slot 3: `ConsoleInput` (WRITE — PRODUCTION ONLY: the authority to
//!   inject bytes into the kernel's console line discipline).
//!
//! Everything else is this program's own work, all from ring 3 on the
//! SHARED virtio core (`userspace/virtio.rs` — the fourth driver on
//! it, and the first with a device-writable queue that must stay
//! stocked): discover the resolved device record, self-map the
//! window, run the §3.1 handshake, build ONE split virtqueue packed
//! into a single owned frame together with its event buffers, post
//! every buffer, arm the MSI-X relay, then run in one of two modes.
//!
//! **Mode selection is a capability probe, not a spawn argument**: a
//! zero-length `SYS_CONSOLE_PUSH` pushes nothing and succeeds only if
//! slot 3 really holds the ConsoleInput authority (ADR-0026).
//!
//! - **Console mode** (production): wait on the relay badge → harvest
//!   the event queue → decode keycodes to ASCII → push the bytes into
//!   the kernel's line discipline. Typing in a QEMU window drives the
//!   shell's `arena>` prompt through the SAME queue the serial port
//!   feeds; neither source displaces the other.
//! - **Service mode** (the m6 suite): serve `INPUT_OP_READ` from a
//!   small decoded-key ring — a keyboard is asynchronous, so keys that
//!   arrive between client calls are buffered, not lost.
//!
//! The device's own batching shapes the harvest: QEMU flushes an event
//! batch only on `EV_SYN`/`SYN_REPORT`, pops ONE descriptor per event,
//! drops the WHOLE batch if the ring cannot absorb it, and raises
//! exactly one interrupt per flushed batch. So the driver posts many
//! buffers, and every wake drains the used ring to its end rather than
//! assuming one event per notification.
//!
//! Exit codes (the diagnostic contract with the m6 suite): 42 clean
//! shutdown (the poison request was served), 80..89 stage failures,
//! 97 console refused, 99 panic.
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
const SLOT_NOTIF: u64 = 2;
/// The console-input authority — present ONLY in the production
/// instance. Its absence is what puts this image in service mode.
const SLOT_CONSOLE: u64 = 3;

/// Owned frame slots: ONE frame holds both the event queue's three
/// ring areas (at the shared core's `ring_offsets`) and the posted
/// event buffers (at [`EVT_BUF_OFF`]) — the ring areas for a 64-entry
/// queue end at 1678, so the second half of the page is free space,
/// and the driver ASSERTS the two never overlap rather than trusting
/// the arithmetic.
const SLOT_FRAME_BASE: u64 = 8;
const FRAMES_TOTAL: usize = 1;

/// The IRQ badge the relay delivers (only the relay notifies this nid).
const IRQ_BADGE_INPUT: u64 = 0x7301;

// ---- the diagnostic exit contract (m6.rs maps every code) --------------------

const EXIT_DEV_INFO: u64 = 80;
const EXIT_MAP: u64 = 81;
const EXIT_HANDSHAKE: u64 = 82;
const EXIT_QUEUE: u64 = 83;
const EXIT_RELAY: u64 = 84;
const EXIT_RECV: u64 = 85;
const EXIT_WAIT: u64 = 86;
const EXIT_REPLY: u64 = 88;
const EXIT_PUSH: u64 = 89;

// ---- virtio-input specifics (the shared 1.0 core lives in userspace/virtio.rs)

/// virtio-input's PCI device id. QEMU calls `virtio_pci_force_virtio_1`
/// on this class, so the ONLY id it ever presents is the modern
/// 0x1040 + 18 = 0x1052 — unlike net/blk/rng there is no transitional
/// alias, and the core's transitional argument is deliberately the
/// same value (nothing else may ever match).
const DEV_ID_INPUT: u64 = 0x1052;

/// The event queue (index 0). Queue 1 is the status queue (LEDs, key
/// repeat) — v1 drives neither, so it is left unenabled rather than
/// set up and ignored.
const QUEUE_EVT: u16 = 0;

/// The queue-size cap this driver asks the core to clamp to. QEMU's
/// virtio-input advertises 64; the device's own number always wins.
const QUEUE_MAX: u16 = 64;

/// One virtio-input event on the wire: `{le16 type, le16 code,
/// le32 value}` — the evdev triple, 8 bytes.
const EVT_BYTES: u64 = 8;

/// How many device-writable event buffers stay posted. QEMU pops one
/// descriptor PER EVENT and drops the entire batch if it cannot place
/// all of it, so an under-stocked ring loses keystrokes silently —
/// 32 absorbs any plausible burst (a keystroke is 3 events).
const EVT_BUFS: u16 = 32;

/// Where the event buffers live inside the ring frame. The ring areas
/// for the largest queue this driver accepts end well below it; the
/// setup asserts that rather than assuming it.
const EVT_BUF_OFF: u64 = 2048;

// ---- evdev (the Linux input event codes QEMU speaks) -------------------------

const EV_KEY: u16 = 1;

/// Key `value`: 0 = release, 1 = press, 2 = autorepeat. Presses and
/// autorepeats produce bytes; releases only update modifier state.
const KEY_RELEASE: u32 = 0;
const KEY_PRESS: u32 = 1;
const KEY_REPEAT: u32 = 2;

const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;

/// The US-ASCII keymap: evdev keycode → byte, unshifted. Index 0 and
/// every unmapped code (escape, control, alt, the function keys, the
/// keypad, everything above 57) is `\0` = "produces no byte" — dropped
/// honestly rather than guessed at. Backspace (14) and tab (15) map to
/// their control bytes; enter (28) to CR, which is exactly what the
/// line discipline treats as end-of-line.
const KEYMAP_LEN: usize = 58;
const KEYMAP_BASE: [u8; KEYMAP_LEN] =
    *b"\x00\x001234567890-=\x08\tqwertyuiop[]\r\x00asdfghjkl;'`\x00\\zxcvbnm,./\x00*\x00 ";
/// The same table with either shift held.
const KEYMAP_SHIFT: [u8; KEYMAP_LEN] =
    *b"\x00\x00!@#$%^&*()_+\x08\tQWERTYUIOP{}\r\x00ASDFGHJKL:\"~\x00|ZXCVBNM<>?\x00*\x00 ";

/// The decoded-key ring: keys that arrive while nobody is asking must
/// survive until someone does (a keyboard is asynchronous — netd's
/// single hold slot was a link-probe simplification, not a model for
/// this device).
const KEY_RING: usize = 64;

/// Defensive bound on re-waits for one batch (a device that raises the
/// interrupt without publishing a used entry is broken; the bound turns
/// that into a typed failure instead of a park the kernel must drain).
const HARVEST_REWAITS: u32 = 64;

fn fail(code: u64, what: &str) -> ! {
    log_line(|o| {
        o.str("inputd: FAIL: ");
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

/// The shared virtio core's typed stage failure → inputd's exit-code
/// contract (the m6 suite maps every code): the five setup stages in
/// order, 80..84.
fn vfail(e: VErr) -> ! {
    match e {
        VErr::DevInfo(r) => fail(EXIT_DEV_INFO, r),
        VErr::Map(r) => fail(EXIT_MAP, r),
        VErr::Handshake(r) => fail(EXIT_HANDSHAKE, r),
        VErr::Queue(r) => fail(EXIT_QUEUE, r),
        VErr::Relay(r) => fail(EXIT_RELAY, r),
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write_str("inputd: PANIC\r\n");
    // SAFETY: thread_exit diverges.
    unsafe { syscall1(SYS_THREAD_EXIT, EXIT_PANIC) };
    loop {
        core::hint::spin_loop();
    }
}

/// The whole driver state (single-threaded image; both service loops
/// own it end to end).
struct Drv {
    q: Queue,
    /// The event buffers' base VA inside the owned ring frame.
    bufs_va: u64,
    /// Their base physical address (what the descriptors carry).
    bufs_phys: u64,
    /// Decoded keys awaiting a consumer (FIFO).
    keys: [u8; KEY_RING],
    head: usize,
    len: usize,
    /// Either shift held — modifier state lives across events.
    shift: bool,
    /// Interrupt-delivered event batches (what the poison reply
    /// reports: the driver's own count of hardware wakes).
    batches: u64,
    /// Events harvested and events that produced a byte.
    events: u64,
    bytes: u64,
    /// Keys dropped because the ring was full (nobody consuming).
    dropped: u64,
    /// Times the spawner abandoned a wait (nobody was typing).
    give_ups: u64,
}

impl Drv {
    /// Post (or re-post) event buffer `id`: ONE device-writable
    /// descriptor over its 8-byte slot, published to the event queue.
    ///
    /// # Safety
    /// The queue's VAs are this driver's own mapped frame;
    /// single-threaded image.
    unsafe fn post(&mut self, id: u16) {
        // SAFETY: method contract.
        unsafe {
            desc_write(
                self.q.desc_va,
                id,
                self.bufs_phys + u64::from(id) * EVT_BYTES,
                EVT_BYTES as u32,
                DESC_F_WRITE,
                0,
            );
            self.q.publish(id);
        }
    }

    /// Buffer one decoded key byte. A full ring means nobody is
    /// consuming: the NEWEST key loses (so what is already queued
    /// stays in typing order) and the loss is counted, never hidden.
    fn push_key(&mut self, b: u8) {
        if self.len == KEY_RING {
            self.dropped += 1;
            return;
        }
        self.keys[(self.head + self.len) % KEY_RING] = b;
        self.len += 1;
        self.bytes += 1;
    }

    /// Take up to `out.len()` buffered keys in order; returns the count.
    fn take(&mut self, out: &mut [u8]) -> usize {
        let n = core::cmp::min(out.len(), self.len);
        for (i, slot) in out.iter_mut().enumerate().take(n) {
            *slot = self.keys[(self.head + i) % KEY_RING];
        }
        self.head = (self.head + n) % KEY_RING;
        self.len -= n;
        n
    }

    /// Decode one evdev event into the key ring (or into modifier
    /// state). Everything this keymap does not cover is ignored.
    fn decode(&mut self, etype: u16, code: u16, value: u32) {
        if etype != EV_KEY {
            return; // EV_SYN and the rest are batching punctuation
        }
        if code == KEY_LEFTSHIFT || code == KEY_RIGHTSHIFT {
            self.shift = value != KEY_RELEASE;
            return;
        }
        if value != KEY_PRESS && value != KEY_REPEAT {
            return; // releases produce no byte
        }
        if (code as usize) >= KEYMAP_LEN {
            return;
        }
        let b = if self.shift {
            KEYMAP_SHIFT[code as usize]
        } else {
            KEYMAP_BASE[code as usize]
        };
        if b != 0 {
            self.push_key(b);
        }
    }

    /// Drain the used ring to its end, decoding every completed event
    /// buffer and immediately re-posting it (the ring must never
    /// shrink — QEMU drops whole batches against a short ring).
    /// Returns the number of events harvested.
    ///
    /// # Safety
    /// Method contract as `post`; volatile ring reads after the
    /// device's completion interrupt.
    unsafe fn harvest(&mut self) -> u32 {
        let mut n = 0u32;
        // SAFETY: method contract.
        unsafe {
            while self.q.used_idx() != self.q.used_seen {
                let slot = self.q.used_seen % self.q.qsz;
                let (id, written) = self.q.used_entry(slot);
                self.q.used_seen = self.q.used_seen.wrapping_add(1);
                if id >= u32::from(EVT_BUFS) {
                    // Outside the posted buffers: honest log, and no
                    // re-post (re-posting an id we never owned would
                    // corrupt the ring).
                    log_line(|o| {
                        o.str("inputd: used entry names descriptor ");
                        o.u64(u64::from(id));
                        o.str(" — outside the posted event buffers");
                    });
                    continue;
                }
                if u64::from(written) >= EVT_BYTES {
                    let p = self.bufs_va + u64::from(id) * EVT_BYTES;
                    let etype = r16(p);
                    let code = r16(p + 2);
                    let value = r32(p + 4);
                    self.decode(etype, code, value);
                    self.events += 1;
                    n += 1;
                }
                self.post(id as u16);
            }
        }
        n
    }

    /// Block until at least one key is buffered: wait on the relay
    /// badge, harvest, repeat. Completions are interrupts, never
    /// polls.
    ///
    /// Returns `Ok(true)` when a key is available and `Ok(false)` when
    /// the SPAWNER sent [`INPUT_BADGE_GIVE_UP`] on the same
    /// notification — the driver cannot distinguish "nobody is typing"
    /// from "nobody has typed YET", so abandoning the wait is the
    /// supervisor's call, never a timeout invented here. `Err(())` =
    /// the wait was refused, an unknown badge arrived, or the device
    /// woke us without publishing anything, repeatedly.
    ///
    /// # Safety
    /// Method contract as `post`.
    unsafe fn wait_for_key(&mut self) -> Result<bool, ()> {
        let mut idle = 0u32;
        while self.len == 0 {
            // SAFETY: wrapper contract.
            let b = unsafe { syscall1(SYS_WAIT, SLOT_NOTIF) };
            if b == INPUT_BADGE_GIVE_UP as i64 {
                self.give_ups += 1;
                return Ok(false);
            }
            if b != IRQ_BADGE_INPUT as i64 {
                log_line(|o| {
                    o.str("inputd: wait returned ");
                    o.i64(b);
                    o.str(", expected the relay badge");
                });
                return Err(());
            }
            self.batches += 1;
            // SAFETY: method contract.
            let got = unsafe { self.harvest() };
            if got == 0 {
                idle += 1;
                if idle > HARVEST_REWAITS {
                    log("inputd: interrupts without event entries");
                    return Err(());
                }
            } else {
                idle = 0;
            }
        }
        Ok(true)
    }
}

/// The driver entry: the spawn protocol's first thread lands here at
/// ring 3 with the grant slots filled. Nothing returns — the service
/// ends only through the poison request's clean exit, a `fail`, or (in
/// console mode) the machine's shutdown.
///
/// # Safety
/// Entered exactly as rngd's `_start`: ring 3, RSP at the derived
/// stack top, the address space the kernel loaded and validated,
/// grants in slots 0..2 (and 3 in production). The body is ABI v1
/// wrappers over this image's own statics and windows — every device
/// and ring touch is volatile, single-threaded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // SAFETY: this is the whole program — ABI v1 wrappers over this
    // image's own statics and its granted/allocated windows, inside
    // the address space the kernel loaded and validated.
    // Single-threaded; every device and ring touch is volatile.
    unsafe {
        log("inputd: starting — ArenaOS input service (M6.3, ADR-0026)");

        // 1. Order-independent discovery + the window map + the §3.1
        //    handshake — the shared virtio core (userspace/virtio.rs).
        //    inputd's demands: a virtio-INPUT function (a mis-spawn
        //    must fail loudly, not drive the wrong device), a
        //    one-entry MSI-X table for the relay, VERSION_1 and
        //    nothing else (virtio-input defines no class feature
        //    bits), and at least the event queue.
        let info = virtio::discover("inputd", "virtio-input", DEV_ID_INPUT, DEV_ID_INPUT, 1)
            .unwrap_or_else(|e| vfail(e));
        let w = virtio::map_window("inputd", "virtio-input", &info, SLOT_MMIO)
            .unwrap_or_else(|e| vfail(e));
        virtio::handshake("inputd", &w, 0, FEATURE_VERSION_1, 1).unwrap_or_else(|e| vfail(e));

        // 2. The ONE owned frame: the event queue's ring areas packed
        //    by the core, plus the event buffers above them.
        //    Allocation pays the phys (the queue registers need it);
        //    self-map consumes the cap (ownership moves to this
        //    address space — teardown exact); the core zeroes the
        //    mapping (rings must START zeroed).
        let mut phys = [0u64; FRAMES_TOTAL];
        let mut va = [0u64; FRAMES_TOTAL];
        virtio::alloc_frames("inputd", SLOT_FRAME_BASE, FRAMES_TOTAL, &mut phys, &mut va)
            .unwrap_or_else(|e| vfail(e));

        // 3. The event queue, through the core: MSI-X entry 0 armed
        //    through the kernel relay BEFORE enable (ring 3 never
        //    touches the table), doorbell resolved, enable
        //    sticky-checked.
        let (q, vec) = virtio::queue_setup(
            "inputd",
            &w,
            &info,
            QUEUE_EVT,
            QUEUE_MAX,
            RingMem::Packed {
                frame_phys: phys[0],
                frame_va: va[0],
            },
            IrqPlan {
                msix_entry: 0,
                slot_notif: SLOT_NOTIF,
                badge: IRQ_BADGE_INPUT,
            },
        )
        .unwrap_or_else(|e| vfail(e));

        // The buffers share the ring frame — so PROVE they fit rather
        // than trusting the arithmetic: the used area ends at
        // used_off + 6 + 8*qsz, and every posted buffer must have a
        // descriptor.
        let (_, _, used_off) = virtio::ring_offsets(q.qsz);
        let rings_end = used_off + 6 + 8 * u64::from(q.qsz);
        let bufs_end = EVT_BUF_OFF + u64::from(EVT_BUFS) * EVT_BYTES;
        if rings_end > EVT_BUF_OFF || bufs_end > 4096 || EVT_BUFS > q.qsz {
            fail(
                EXIT_QUEUE,
                "the event buffers do not fit the ring frame beside the rings",
            );
        }

        let mut drv = Drv {
            q,
            bufs_va: va[0] + EVT_BUF_OFF,
            bufs_phys: phys[0] + EVT_BUF_OFF,
            keys: [0u8; KEY_RING],
            head: 0,
            len: 0,
            shift: false,
            batches: 0,
            events: 0,
            bytes: 0,
            dropped: 0,
            give_ups: 0,
        };

        // 4. Stock the event queue BEFORE DRIVER_OK: the device only
        //    starts sending at DRIVER_OK, and it drops a whole batch
        //    it cannot place. Buffers first, then the go-ahead.
        for id in 0..EVT_BUFS {
            drv.post(id);
        }
        virtio::driver_ok(&w);
        // One line, well inside WRITE_MAX=256 (Out::push drops past
        // the limit silently — the line budget is an invariant).
        log_line(|o| {
            o.str("inputd: virtio-input ready — DRIVER_OK, eventq size ");
            o.u64(u64::from(drv.q.qsz));
            o.str(", ");
            o.u64(u64::from(EVT_BUFS));
            o.str(" event buffers posted, ring frame ");
            o.hex(phys[0]);
            o.str("; msix entry 0 → relay vector ");
            o.i64(vec);
        });

        // 5. Which service is this? A zero-length push asks the
        //    kernel whether slot 3 really holds the ConsoleInput
        //    authority — the capability IS the mode (ADR-0026).
        if syscall3(SYS_CONSOLE_PUSH, SLOT_CONSOLE, 0, 0) >= 0 {
            console_loop(&mut drv);
        }
        service_loop(&mut drv);
    }
}

/// PRODUCTION: decode keystrokes into the kernel's console line
/// discipline forever. Never returns — the machine outlives this loop.
///
/// # Safety
/// As `_start`: the grants are in place and slot 3 has been PROVEN to
/// carry the console authority by the probe above.
unsafe fn console_loop(drv: &mut Drv) -> ! {
    log("inputd: console mode — keystrokes feed the shell's line discipline (serial stays live)");
    let mut buf = [0u8; CONSOLE_PUSH_MAX as usize];
    loop {
        // SAFETY: function contract; the wait blocks on the relay
        // notification and the harvest reads this driver's own frame.
        unsafe {
            match drv.wait_for_key() {
                Ok(_) => {}
                Err(()) => fail(EXIT_WAIT, "the relay wait or harvest failed"),
            }
            loop {
                let n = drv.take(&mut buf);
                if n == 0 {
                    break;
                }
                let r = syscall3(
                    SYS_CONSOLE_PUSH,
                    SLOT_CONSOLE,
                    buf.as_ptr() as u64,
                    n as u64,
                );
                if r != n as i64 {
                    log_line(|o| {
                        o.str("inputd: console push of ");
                        o.u64(n as u64);
                        o.str(" byte(s) returned ");
                        o.i64(r);
                    });
                    fail(EXIT_PUSH, "SYS_CONSOLE_PUSH refused");
                }
            }
        }
    }
}

/// THE SUITE'S INSTANCE: serve decoded keys over the endpoint. Ends
/// through the poison request's clean exit.
///
/// # Safety
/// As `_start`.
unsafe fn service_loop(drv: &mut Drv) -> ! {
    log("inputd: service mode — no console authority, serving decoded keys on the endpoint");
    let mut msg = [0u64; 3];
    let mut out = [0u8; MSG_BYTES];
    loop {
        // SAFETY: function contract; `msg`/`out` are this image's own
        // buffers and the endpoint cap is the granted slot.
        unsafe {
            let r = syscall3(SYS_IPC_RECV, SLOT_EP, msg.as_mut_ptr() as u64, 0);
            if r < 0 {
                log_line(|o| {
                    o.str("inputd: recv returned ");
                    o.i64(r);
                });
                fail(EXIT_RECV, "SYS_IPC_RECV refused");
            }
            let (want, op, landed) = (msg[0], msg[1], msg[2]);
            if landed != CAP_NONE {
                // This service lends nothing and takes nothing: a cap
                // that rode in is discarded, never silently kept.
                let _ = syscall1(SYS_CAP_DESTROY, landed);
            }
            match op {
                INPUT_OP_SHUTDOWN => {
                    log_line(|o| {
                        o.str("inputd: shutdown requested after ");
                        o.u64(drv.batches);
                        o.str(" interrupt-delivered batch(es), ");
                        o.u64(drv.events);
                        o.str(" event(s), ");
                        o.u64(drv.bytes);
                        o.str(" key byte(s), ");
                        o.u64(drv.give_ups);
                        o.str(" abandoned wait(s) — replying and exiting");
                    });
                    let rr = syscall5(SYS_IPC_REPLY, SLOT_EP, INPUT_S_OK, drv.batches, CAP_NONE, 0);
                    if rr < 0 {
                        fail(EXIT_REPLY, "the shutdown reply refused");
                    }
                    // SAFETY: thread_exit diverges; 42 is the clean-exit code.
                    syscall1(SYS_THREAD_EXIT, EXIT_OK);
                }
                INPUT_OP_READ => {
                    if !(1..=INPUT_READ_MAX).contains(&want) {
                        reply_err(INPUT_S_BAD_LEN);
                        continue;
                    }
                    // Block until the keyboard has something to say —
                    // on an interrupt, never a poll. The spawner may
                    // call the wait off (nobody is typing), and that
                    // is answered honestly rather than hung on.
                    match drv.wait_for_key() {
                        Ok(true) => {}
                        Ok(false) => {
                            log("inputd: the wait was called off — nobody is typing");
                            reply_err(INPUT_S_NO_KEYS);
                            continue;
                        }
                        Err(()) => fail(EXIT_WAIT, "the relay wait or harvest failed"),
                    }
                    let n = drv.take(&mut out[..want as usize]);
                    let rr = syscall5(
                        SYS_IPC_REPLY,
                        SLOT_EP,
                        INPUT_S_OK,
                        n as u64,
                        CAP_NONE,
                        out.as_ptr() as u64,
                    );
                    if rr < 0 {
                        fail(EXIT_REPLY, "SYS_IPC_REPLY refused");
                    }
                    log_line(|o| {
                        o.str("inputd: READ → ");
                        o.u64(n as u64);
                        o.str(" decoded key byte(s) delivered inline (batch #");
                        o.u64(drv.batches);
                        o.str(")");
                    });
                }
                _ => reply_err(INPUT_S_BAD_OP),
            }
        }
    }
}

/// Reply with a typed error status.
fn reply_err(status: u64) {
    // SAFETY: ABI v1 wrapper; the endpoint cap is the granted slot.
    let rr = unsafe { syscall5(SYS_IPC_REPLY, SLOT_EP, status, 0, CAP_NONE, 0) };
    if rr < 0 {
        fail(EXIT_REPLY, "the error reply refused");
    }
}
