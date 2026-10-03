//! The console input service v1 (M4.6, ADR-0020): COM1 RX, IRQ-driven,
//! with a kernel-side line discipline.
//!
//! The path a typed byte walks: the 16550 raises its interrupt line →
//! IOAPIC pin 4 (ISA IRQ4, edge) → LAPIC → IDT vector 33
//! (`arena_irq_serial_stub`) → [`rx_isr`] drains the RBR → [`feed`]
//! runs the line discipline → a committed line wakes the parked
//! `SYS_CONSOLE_READ` reader.
//!
//! [`feed`] is `pub` on purpose: it is *the same function* the ISR
//! calls, so the M4.6 suite drives the real line discipline (edit,
//! commit, queue, wake) without a real UART — and the harness drives
//! the real UART through QEMU's stdio chardev. Two entry points, one
//! code path, no test-only twin.
//!
//! Discipline rules (all decided in ADR-0020):
//! - printable ASCII (0x20..=0x7E) accumulates into the edit buffer
//!   (bound [`LINE_MAX`]; excess bytes are dropped and counted),
//! - 0x08/0x7F erase one byte,
//! - CR or LF commits a NONEMPTY line into the ring queue and resets
//!   the edit buffer; empty lines are consumed, never queued,
//! - every accepted byte is echoed to TX as it arrives; a commit
//!   echoes CRLF (the terminal shows what the user typed),
//! - the queue holds [`QUEUE_DEPTH`] lines; when full, the OLDEST line
//!   is dropped and counted — the ISR is never back-pressured,
//! - one reader at a time: a line committed while a reader is parked
//!   is RESERVED for it (a second reader gets `STATUS_BUSY`, never a
//!   stolen line), and a second parked reader is refused outright.

use crate::arch::x86_64::idt;
use crate::arch::x86_64::syscall::{STATUS_BUSY, Status};
use crate::drivers::serial;
use crate::sched;
use crate::sync::{SyncCell, without_interrupts};

/// Longest line the discipline commits (and the longest
/// `SYS_CONSOLE_READ` accepts as `max`). A shell command longer than
/// this is a mistake, not a use case.
pub const LINE_MAX: usize = 120;
/// Complete lines waiting for a reader. Full = drop the oldest.
pub const QUEUE_DEPTH: usize = 4;

/// "No reader parked" sentinel (thread ids are small positive ints).
const NO_TID: u64 = u64::MAX;

/// Everything the discipline owns. Single CPU; every access is under
/// IF=0 (`without_interrupts`) — the ISR and syscall paths alike.
struct Console {
    /// The line being typed (not yet committed).
    edit: [u8; LINE_MAX],
    edit_len: usize,
    /// Committed lines, ring order from `head`.
    q: [[u8; LINE_MAX]; QUEUE_DEPTH],
    qlen: [usize; QUEUE_DEPTH],
    head: usize,
    count: usize,
    /// The one parked `SYS_CONSOLE_READ` reader (reservation holder).
    waiter: u64,
    // --- counters (machine-state evidence for the suite) ---
    /// Bytes accepted from the device (every byte the ISR read).
    rx_bytes: u64,
    /// Lines committed to the queue.
    lines_in: u64,
    /// Committed lines lost to a full queue (oldest-first).
    dropped_lines: u64,
    /// Edit-buffer overflow bytes dropped mid-line.
    dropped_bytes: u64,
    /// Reads that got less than the whole line (caller's `max` too
    /// small; the remainder is discarded, never re-queued).
    truncations: u64,
    /// `SYS_CONSOLE_READ` invocations (immediate + blocking).
    reads: u64,
    /// Reads that parked the caller (blocked on an empty queue).
    blocks: u64,
}

const CONSOLE_INIT: Console = Console {
    edit: [0; LINE_MAX],
    edit_len: 0,
    q: [[0; LINE_MAX]; QUEUE_DEPTH],
    qlen: [0; QUEUE_DEPTH],
    head: 0,
    count: 0,
    waiter: NO_TID,
    rx_bytes: 0,
    lines_in: 0,
    dropped_lines: 0,
    dropped_bytes: 0,
    truncations: 0,
    reads: 0,
    blocks: 0,
};

static CONSOLE: SyncCell<Console> = SyncCell::new(CONSOLE_INIT);

/// The counters, as one copyable snapshot (the suite asserts deltas).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ConsoleStats {
    pub rx_bytes: u64,
    pub lines_in: u64,
    pub dropped_lines: u64,
    pub dropped_bytes: u64,
    pub truncations: u64,
    pub reads: u64,
    pub blocks: u64,
}

pub fn stats() -> ConsoleStats {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe {
            let c = &*CONSOLE.get();
            ConsoleStats {
                rx_bytes: c.rx_bytes,
                lines_in: c.lines_in,
                dropped_lines: c.dropped_lines,
                dropped_bytes: c.dropped_bytes,
                truncations: c.truncations,
                reads: c.reads,
                blocks: c.blocks,
            }
        }
    })
}

/// Lines currently queued (non-blocking peek for the suite's teardown).
pub fn pending_lines() -> usize {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (*CONSOLE.get()).count }
    })
}

/// Arm the console input path: install the RX hook in the vector-33
/// handler, then unmask the UART's receive interrupt. Call exactly
/// once, after ExitBootServices, with the IDT live and IOAPIC pin 4
/// routed to `SERIAL_RX_VECTOR` (the boot sequence's timer-chain
/// reclaim block does all three together — ADR-0011/0020).
pub fn init() {
    idt::set_serial_hook(Some(rx_isr));
    // M6.4 (ADR-0027): the output mirror's deferred wake, now one tick
    // TASK among several (M7.0) rather than the sole owner of the
    // auxiliary hook — the timer facility needs the same service, and
    // "whoever registers last wins" is not a mechanism. Harmless while
    // no channel is attached: it reads one flag and returns.
    if let Err(e) = crate::tick::register(mirror_flush) {
        crate::log::log_error!("console", "tick task registration failed: {e}");
    }
    // SAFETY: ring 0; the console subsystem is the sole COM1 RX owner
    // from here on; EBS has passed (caller contract), so the
    // interrupt-driven model is legal now.
    unsafe { serial::enable_rx_interrupt() };
}

/// The vector-33 hook: drain the receiver, feed the discipline. Runs
/// in interrupt context (IF=0, caller-saved registers saved by the
/// stub, EOI already sent). Never switches threads — `commit`'s wake
/// only enqueues.
extern "C" fn rx_isr() {
    // SAFETY: interrupt context owns the RX drain (init contract);
    // every RBR read follows an observed LSR.DR.
    unsafe {
        while serial::rx_ready() {
            feed(serial::rx_byte());
        }
    }
}

/// The line discipline's byte entry point — exactly what [`rx_isr`]
/// calls per received byte; `pub` so the suite drives the same path
/// (see the module header).
pub fn feed(b: u8) {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let c = &mut *CONSOLE.get();
            c.rx_bytes += 1;
            match b {
                0x08 | 0x7F => {
                    if c.edit_len > 0 {
                        c.edit_len -= 1;
                        echo_erase();
                    }
                }
                b'\r' | b'\n' => {
                    echo(b'\r');
                    echo(b'\n');
                    if c.edit_len > 0 {
                        commit(c);
                    }
                }
                0x20..=0x7E => {
                    if c.edit_len < LINE_MAX {
                        c.edit[c.edit_len] = b;
                        c.edit_len += 1;
                        echo(b);
                    } else {
                        c.dropped_bytes += 1;
                    }
                }
                _ => {} // other control bytes: not input, ignored
            }
        }
    });
}

/// Commit the edit buffer into the ring queue and wake the reserved
/// reader, if one is parked. Caller holds the Console under IF=0.
fn commit(c: &mut Console) {
    if c.count == QUEUE_DEPTH {
        // Oldest line loses — the ISR must never block or drop the NEW
        // input on account of an absent reader.
        c.head = (c.head + 1) % QUEUE_DEPTH;
        c.count -= 1;
        c.dropped_lines += 1;
    }
    let slot = (c.head + c.count) % QUEUE_DEPTH;
    c.q[slot][..c.edit_len].copy_from_slice(&c.edit[..c.edit_len]);
    c.qlen[slot] = c.edit_len;
    c.count += 1;
    c.lines_in += 1;
    c.edit_len = 0;
    if c.waiter != NO_TID {
        // The line is RESERVED: `waiter` stays set until the woken
        // reader pops it, so no second reader can steal it between the
        // wake and the reader's resume. Wake only enqueues (no switch
        // inside the ISR); a second commit while the waiter is already
        // Ready returns Err — harmless, ignored.
        let _ = sched::wake(c.waiter);
    }
}

/// The blocking read behind `SYS_CONSOLE_READ`: pop the next complete
/// line into `buf` (at most [`LINE_MAX`] bytes; shorter `buf`
/// truncates and counts), parking the caller while the queue is empty.
/// `Err(STATUS_BUSY)` = another reader holds the reservation.
///
/// Blocking rides the ADR-0018 machinery (`block_current` with its
/// GS-side capture/normalize/restore), so parking inside the syscall
/// dispatcher is the proven-safe shape; the wake arrives from
/// interrupt context, which `sched::wake` supports by construction
/// (enqueue-only, IF=0).
pub fn read_line(buf: &mut [u8]) -> Result<usize, Status> {
    // Phase 1 (IF=0): take a queued line now, or park as the reader.
    let immediate = without_interrupts(|| -> Result<Option<usize>, Status> {
        // SAFETY: single writer under IF=0.
        unsafe {
            let c = &mut *CONSOLE.get();
            c.reads += 1;
            if c.waiter == NO_TID && c.count > 0 {
                return Ok(Some(pop_into(c, buf)));
            }
            if c.waiter != NO_TID {
                // Queued lines (if any) are reserved for the parked
                // reader; the console serves exactly one reader.
                return Err(STATUS_BUSY);
            }
            c.waiter = sched::current_thread_id();
            Ok(None)
        }
    })?;
    if let Some(n) = immediate {
        return Ok(n);
    }

    // Phase 2: park. Only `commit` wakes a console reader, and it
    // always leaves a line queued AND keeps the reservation — so on
    // resume the pop cannot fail (the same invariant ipc::wait enforces
    // against a badge-less wake).
    // SAFETY: single writer under IF=0.
    without_interrupts(|| unsafe { (*CONSOLE.get()).blocks += 1 });
    sched::block_current();
    without_interrupts(|| -> Result<usize, Status> {
        // SAFETY: single writer under IF=0.
        unsafe {
            let c = &mut *CONSOLE.get();
            if c.count == 0 {
                crate::halt::halt_machine("console: read woken without a line");
            }
            let n = pop_into(c, buf);
            c.waiter = NO_TID; // reservation consumed
            Ok(n)
        }
    })
}

/// Pop the oldest queued line into `buf`, truncating (and counting) if
/// `buf` is shorter. Caller holds the Console under IF=0 with
/// `count > 0`.
fn pop_into(c: &mut Console, buf: &mut [u8]) -> usize {
    let slot = c.head;
    let len = c.qlen[slot];
    let n = len.min(buf.len());
    buf[..n].copy_from_slice(&c.q[slot][..n]);
    if n < len {
        c.truncations += 1;
    }
    c.head = (c.head + 1) % QUEUE_DEPTH;
    c.count -= 1;
    n
}

fn echo(b: u8) {
    // SAFETY: the console subsystem is COM1's owner (init contract);
    // putc is a bounded spin, legal in interrupt context.
    unsafe { serial::putc(b) };
}

fn echo_erase() {
    echo(0x08);
    echo(b' ');
    echo(0x08);
}

// ---- the output mirror (M6.4, ADR-0027) -------------------------------------
//
// The console has always had exactly one output device: COM1, written
// byte-by-byte by `echo` and by `log`. A second CHANNEL (the
// virtio-console port `consoled` drives) cannot be a second writer —
// ring 3 must not touch the UART, and the kernel must not know what a
// virtqueue is. So the kernel offers what it already owns: a mirror of
// the byte stream it emits.
//
// One holder of the `ConsoleOutput` capability ATTACHES a notification
// (exactly like an IRQ relay: nid + badge), which turns mirroring on.
// From then on every byte the kernel writes to the console is also
// appended to this ring, and the attacher is woken when the ring goes
// from empty to non-empty. It drains with `SYS_CONSOLE_PULL` and
// writes the bytes wherever it likes. Nothing about the serial path
// changes — it stays the kernel's own, including for panics.
//
// Off by default and off again when the attacher dies: a machine with
// no console channel does not pay for a ring it never reads.

/// Mirror ring capacity. One boot-time log line is ~100 bytes and the
/// shell's longest output (`help`) is under 256, so 4 KiB is dozens of
/// lines of slack for a driver that is woken on the first byte. Full =
/// drop the OLDEST byte and count it: the kernel's own output path
/// must never stall on a userspace channel that stopped draining.
const MIRROR_CAP: usize = 4096;

/// "No output channel attached" sentinel.
const NO_OWNER: u64 = 0;

struct Mirror {
    buf: [u8; MIRROR_CAP],
    head: usize,
    count: usize,
    /// The attacher's pid (0 = detached), its notification and badge.
    owner: u64,
    nid: u32,
    badge: u64,
    /// Edge-trigger latch: set when the attacher has been notified and
    /// not yet drained the ring to empty. Prevents a notify per byte.
    notified: bool,
    /// A wake is OWED to the attacher. Raised by the tap, delivered by
    /// [`mirror_flush`] on the timer tick — never from the tap itself
    /// (see the module note on `mirror_out`).
    wake_owed: bool,
    // --- counters (machine-state evidence for the suite) ---
    bytes_in: u64,
    bytes_out: u64,
    dropped: u64,
    notifies: u64,
}

/// Is a channel attached? A plain atomic OUTSIDE the cell so the tap
/// can answer "no" with one relaxed load and no interrupt juggling.
///
/// This matters more than it looks: `mirror_out` runs on EVERY byte the
/// kernel prints, and a boot prints tens of thousands. Entering an
/// irqsave critical section (pushfq/cli/popfq) per byte to discover
/// that nobody is listening cost enough emulated time to perturb the
/// m2 suite's TSC calibration re-measurement — a feature nobody is
/// using must cost nothing.
static MIRROR_ATTACHED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

static MIRROR: SyncCell<Mirror> = SyncCell::new(Mirror {
    buf: [0; MIRROR_CAP],
    head: 0,
    count: 0,
    owner: NO_OWNER,
    nid: 0,
    badge: 0,
    notified: false,
    wake_owed: false,
    bytes_in: 0,
    bytes_out: 0,
    dropped: 0,
    notifies: 0,
});

/// Snapshot of the mirror's counters (`m6`'s evidence, and the shell's
/// future `console` command).
#[derive(Clone, Copy)]
pub struct MirrorStats {
    pub attached: bool,
    pub owner: u64,
    pub queued: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub dropped: u64,
    pub notifies: u64,
}

pub fn mirror_stats() -> MirrorStats {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        let m = unsafe { &*MIRROR.get() };
        MirrorStats {
            attached: m.owner != NO_OWNER,
            owner: m.owner,
            queued: m.count,
            bytes_in: m.bytes_in,
            bytes_out: m.bytes_out,
            dropped: m.dropped,
            notifies: m.notifies,
        }
    })
}

/// Attach an output channel: `owner` will be woken on `nid` with
/// `badge` whenever mirrored output arrives. Turns mirroring ON.
/// `Err(STATUS_BUSY)` = a channel is already attached (v1 is
/// single-channel, exactly like the single-reader input side).
pub fn attach_output(owner: u64, nid: u32, badge: u64) -> Result<(), Status> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        let m = unsafe { &mut *MIRROR.get() };
        if m.owner != NO_OWNER {
            return Err(STATUS_BUSY);
        }
        m.owner = owner;
        m.nid = nid;
        m.badge = badge;
        m.head = 0;
        m.count = 0;
        m.notified = false;
        m.wake_owed = false;
        MIRROR_ATTACHED.store(true, core::sync::atomic::Ordering::Relaxed);
        Ok(())
    })
}

/// Detach the channel owned by `pid` (its process is being destroyed).
/// Returns true if one was attached — `proc::destroy` logs the sweep,
/// exactly as it does for relay vectors.
pub fn detach_output_by_owner(pid: u64) -> bool {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        let m = unsafe { &mut *MIRROR.get() };
        if m.owner != pid {
            return false;
        }
        m.owner = NO_OWNER;
        m.nid = 0;
        m.badge = 0;
        m.head = 0;
        m.count = 0;
        m.notified = false;
        m.wake_owed = false;
        MIRROR_ATTACHED.store(false, core::sync::atomic::Ordering::Relaxed);
        true
    })
}

/// The tap: every byte the kernel writes to the console passes here.
/// Cheap and silent when no channel is attached (the common case, and
/// every boot before `consoled` starts).
///
/// Called from `drivers::serial::putc`, which runs in EVERY context
/// this kernel has — syscall, interrupt, panic, and inside the
/// scheduler's own log lines. So this function does exactly one thing:
/// append. It must never call `ipc::notify`, because notify wakes a
/// thread, and waking a thread from underneath the scheduler (which is
/// where a `[sched]` log line puts you) re-enters it. The wake is
/// OWED here and PAID in [`mirror_flush`] from the timer tick — an
/// ordinary interrupt context that is allowed to wake threads.
///
/// Discovered the hard way: the first cut notified from here and
/// wedged the machine mid-boot (ADR-0027, Downsides accepted).
#[inline(always)]
pub fn mirror_out(b: u8) {
    // The fast path is INLINED into `putc` and is three instructions:
    // load, test, branch. Not an optimization for its own sake — an
    // out-of-line call here, taken on every byte of a boot's tens of
    // thousands, measurably perturbed the m2 suite's TSC-vs-PIT
    // calibration re-measurement (a 1-in-3 failure, bisected to
    // exactly this call). A tap nobody has attached to must be
    // invisible, in time as well as in behavior.
    if !MIRROR_ATTACHED.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    mirror_out_slow(b);
}

/// The attached path, deliberately out of line so the common case
/// stays three instructions.
#[inline(never)]
fn mirror_out_slow(b: u8) {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        let m = unsafe { &mut *MIRROR.get() };
        if m.owner == NO_OWNER {
            return;
        }
        if m.count == MIRROR_CAP {
            m.head = (m.head + 1) % MIRROR_CAP;
            m.count -= 1;
            m.dropped += 1;
        }
        let slot = (m.head + m.count) % MIRROR_CAP;
        m.buf[slot] = b;
        m.count += 1;
        m.bytes_in += 1;
        if !m.notified {
            m.notified = true;
            m.wake_owed = true;
        }
    });
}

/// Pay any wake the tap owes the attached channel. Installed as the
/// timer's auxiliary hook by [`init`], so the latency between a byte
/// reaching the console and the channel being woken is at most one
/// tick (10 ms at 100 Hz) — invisible on a console, and the price of
/// never waking a thread from inside `putc`.
///
/// `ipc::notify` latches the badge, so a wake is never lost between
/// the attacher's last pull and its next wait.
extern "C" fn mirror_flush() {
    // Same fast path: this runs on every timer tick forever.
    if !MIRROR_ATTACHED.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let wake = without_interrupts(|| {
        // SAFETY: single writer under IF=0 (interrupt context).
        let m = unsafe { &mut *MIRROR.get() };
        if m.owner == NO_OWNER || !m.wake_owed {
            return None;
        }
        m.wake_owed = false;
        m.notifies += 1;
        Some((m.nid, m.badge))
    });
    if let Some((nid, badge)) = wake {
        // A dead notification is not worth halting for: the channel is
        // torn down when its owner is destroyed anyway.
        let _ = crate::ipc::notify(nid, badge);
    }
}

/// Drain up to `buf.len()` mirrored bytes. Non-blocking by design: the
/// attacher has exactly one thing to block on (its notification), and
/// it also has a device to service — a second blocking source would
/// make those two mutually exclusive.
///
/// Returns 0 when nothing is queued, and re-arms the notification at
/// that moment (edge-triggered from empty).
pub fn pull_output(buf: &mut [u8]) -> usize {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        let m = unsafe { &mut *MIRROR.get() };
        let n = core::cmp::min(buf.len(), m.count);
        for (i, out) in buf[..n].iter_mut().enumerate() {
            *out = m.buf[(m.head + i) % MIRROR_CAP];
        }
        m.head = (m.head + n) % MIRROR_CAP;
        m.count -= n;
        m.bytes_out += n as u64;
        if m.count == 0 {
            // Empty: the next byte owes a new wake.
            m.notified = false;
        }
        n
    })
}

/// Is `pid` the attached output channel? The syscall path's ownership
/// check — the capability proves authority, this proves identity.
pub fn output_owner_is(pid: u64) -> bool {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        let m = unsafe { &*MIRROR.get() };
        m.owner == pid
    })
}
