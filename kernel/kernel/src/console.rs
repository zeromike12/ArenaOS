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
