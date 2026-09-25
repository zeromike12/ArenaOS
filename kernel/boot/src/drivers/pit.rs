//! Intel 8254 Programmable Interval Timer (PIT) — channel 0 only.
//!
//! Ports: 0x40 = channel 0 data, 0x43 = mode/command register (write) /
//! read-back trigger. Channel 0's OUT line is wired to the interrupt
//! fabric (on this platform: IOAPIC pin 0 → LAPIC vector 32; see the M1
//! handoff findings in docs/ARCHITECTURE.md).
//!
//! Mode choices are driven by measured QEMU behavior (hw/timer/i8254*.c):
//! * Mode 3 (square wave) is the ONLY periodic mode whose OUT line is high
//!   for a usable half-period — mode 2 emits 1 ns pulses that the level/
//!   edge IRQ plumbing may not reliably turn into interrupts. Firmware uses
//!   mode 3 too. All production ticking therefore uses mode 3.
//! * Mode 1 (hardware-retriggerable one-shot) leaves OUT=1 permanently once
//!   the count expires — a pollable completion flag via the read-back
//!   command (0xC2: latch ch0 status + count). QEMU starts counting at
//!   count-load time regardless of gate (pit_load_count); mode command bit 0
//!   is BCD and stays 0 (binary counting).
//! * Mode 2 gives the cleanest linear count-down for calibration sampling
//!   (`pit_get_count`: count − elapsed), used by `calibrate_tsc_window`.
//!
//! References: Intel 8254 datasheet; QEMU i8254 emulation (behavior notes
//! above were verified against QEMU 11.0.2 source and on the virtual
//! machine by the M2.2 tests).

use crate::arch::x86_64::{inb, outb, read_tsc};

pub const PORT_CH0: u16 = 0x40;
pub const PORT_MODE: u16 = 0x43;

/// Nominal PIT oscillator frequency (Hz) — the 8254 input clock.
pub const OSCILLATOR_HZ: u64 = 1_193_182;

// Mode/command register encoding (8254 datasheet §mode command):
// bits 7-6 channel (00 = ch0, 11 = read-back), bits 5-4 access
// (01 = latch, 11 = LSB then MSB), bits 3-1 mode, bit 0 BCD (always 0).
const CMD_CH0_LATCH: u8 = 0x00;
/// Channel 0, access = LSB then MSB, BCD = 0 (binary).
const CMD_CH0_LOHI: u8 = 0x30;
/// Read-back: latch status+count of channel 0 only (bits 7-6 = 11,
/// bit 5 = 0 → count latched, bit 4 = 0 → status latched, bits 3-2 = 01).
const CMD_READBACK_CH0: u8 = 0xC2;
/// Mode numbers as they appear in the command byte (bits 3-1). Modes 4/5
/// accept mode bits 100 or 110; we only use 1, 2, 3.
const MODE1_BITS: u8 = 0x02;
const MODE2_BITS: u8 = 0x04;
const MODE3_BITS: u8 = 0x06;

/// Read-back status byte: bit 7 = OUT pin level, bit 6 = null count,
/// bits 5-4 = access mode, bits 3-1 = operating mode, bit 0 = BCD.
pub const STATUS_OUT: u8 = 0x80;

/// Program channel 0 for periodic interrupts at approximately `hz`
/// (square-wave mode 3, gate high, LSB+MSB count load). The actual rate is
/// `OSCILLATOR_HZ / divisor` rounded to the integer divisor.
///
/// # Safety
/// Reprograms the platform tick source; caller must own the PIT (we do,
/// from timekeeping init onward) and be IF-controlled as appropriate.
pub unsafe fn set_periodic_hz(hz: u32) {
    let divisor = pit_divisor_for_hz(hz);
    set_ch0(CMD_CH0_LOHI | MODE3_BITS, divisor);
}

/// Program channel 0 as a one-shot: after `counts` oscillator ticks, OUT
/// goes (and stays) high — observable via [`read_ch0_status`]. Counting
/// starts at the count load (QEMU `pit_load_count` semantics).
///
/// # Safety
/// As [`set_periodic_hz`]; also stops the periodic tick until reprogrammed.
pub unsafe fn set_oneshot(counts: u16) {
    // SAFETY: mode command then LSB+MSB count load; caller owns the PIT.
    set_ch0(CMD_CH0_LOHI | MODE1_BITS, counts);
}

/// Put channel 0 into mode 2 with a full-period count — linear count-down,
/// the cleanest basis for calibration sampling. Does NOT reliably drive
/// interrupts in QEMU (1 ns OUT pulses): callers must run IF-controlled and
/// reprogram for ticking afterwards.
///
/// # Safety
/// As [`set_oneshot`].
pub unsafe fn set_calibration_mode() {
    // SAFETY: port writes; count 0 loads as 65536 per 8254 semantics.
    set_ch0(CMD_CH0_LOHI | MODE2_BITS, 0);
}

/// Latch and read channel 0's current count (16-bit). In mode 2/1 this
/// decrements linearly at OSCILLATOR_HZ; in mode 3 it is a sawtooth with
/// the programmed period (QEMU: `count − (2·elapsed) mod count`).
///
/// # Safety
/// Port I/O; the latch protocol is two sequential reads of 0x40 and must
/// not be interleaved with other ch0 reads (single-CPU, IF=0 context).
pub unsafe fn latch_count_ch0() -> u16 {
    unsafe {
        outb(PORT_MODE, CMD_CH0_LATCH);
        let lo = inb(PORT_CH0) as u16;
        let hi = inb(PORT_CH0) as u16;
        lo | (hi << 8)
    }
}

/// Latch and read channel 0's status byte (read-back command). The count is
/// latched too, so a following [`latch_count_ch0`]-style pair of reads
/// returns the value captured at the same instant.
///
/// # Safety
/// Port I/O; single-CPU, IF=0 context as above.
pub unsafe fn read_ch0_status() -> u8 {
    unsafe {
        outb(PORT_MODE, CMD_READBACK_CH0);
        let status = inb(PORT_CH0);
        // The read-back latched the count too; drain both bytes so later
        // latch commands see fresh values (QEMU keeps `count_latched` sticky
        // until the latched pair is fully read).
        let _ = inb(PORT_CH0);
        let _ = inb(PORT_CH0);
        status
    }
}

/// Integer divisor for a target rate (clamped to the 16-bit counter range;
/// divisor 0 means 65536 per 8254 semantics).
pub fn pit_divisor_for_hz(hz: u32) -> u16 {
    let d = OSCILLATOR_HZ / hz.max(1) as u64;
    d.clamp(2, 65535) as u16
}

fn set_ch0(cmd: u8, divisor: u16) {
    // SAFETY: caller contract of the public fns routes here; port writes in
    // the documented order (mode command, then LSB, then MSB).
    unsafe {
        outb(PORT_MODE, cmd);
        outb(PORT_CH0, divisor as u8);
        outb(PORT_CH0, (divisor >> 8) as u8);
    }
}

/// One TSC-frequency measurement window (mode-2 linear countdown):
/// sample (count, TSC), wait until at least `min_counts` oscillator ticks
/// have elapsed (wrap-aware), sample again, and return
/// (count_delta, tsc_delta).
///
/// # Safety
/// PIT must be in calibration mode; IF must be controlled by the caller
/// (handler interleaving would skew the paired samples).
pub unsafe fn calibrate_tsc_window(min_counts: u32) -> (u32, u64) {
    const PERIOD: u32 = 65536; // count 0 loaded → 65536 per 8254 semantics
    unsafe {
        let c_start = latch_count_ch0() as u32;
        let t0 = read_tsc();
        let mut wraps = 0i64;
        let mut prev = c_start;
        let mut spin = 0u64;
        let mut elapsed = 0i64;
        // 400M iterations ≈ seconds on TCG — far beyond the ~10 ms windows
        // we request; a spin-out returns a short (still nonzero) window
        // rather than hanging.
        while elapsed < min_counts as i64 && spin < 400_000_000 {
            spin += 1;
            let c = latch_count_ch0() as u32;
            if c > prev {
                wraps += 1; // countdown crossed its reload point
            }
            prev = c;
            // Displayed count c corresponds to elapsed-since-load ≡ -c;
            // between two samples: Δ = wraps·PERIOD + (c_start − c), which
            // is correct for partial first/last periods (signed on purpose).
            elapsed = wraps * i64::from(PERIOD) + i64::from(c_start) - i64::from(c);
        }
        // Final tight pair: count first, then TSC (same ordering as the
        // (c_start, t0) pair, so latch latency mostly cancels).
        let c_end = latch_count_ch0() as u32;
        let t1 = read_tsc();
        let total = wraps * i64::from(PERIOD) + i64::from(c_start) - i64::from(c_end);
        (total as u32, t1 - t0)
    }
}
