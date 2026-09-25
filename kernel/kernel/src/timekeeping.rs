//! Timekeeping (M2.2): the platform tick source and the monotonic clock.
//!
//! Two cooperating pieces of hardware, one calibrated fact:
//!
//! * **PIT channel 0** (`drivers/pit.rs`) — the *real-time* reference. Its
//!   oscillator (1 193 182 Hz) is what the word "microsecond" ultimately
//!   means on this machine. `init()` leaves it ticking periodically at
//!   [`KERNEL_TICK_HZ`] in square-wave mode 3, which is both what the
//!   firmware used and the only QEMU periodic mode whose OUT line reliably
//!   drives the IOAPIC→LAPIC interrupt path (measured, see pit.rs header).
//! * **TSC** — the fast monotonic counter (`rdtsc`, no I/O cost). Its rate
//!   is unknown a priori, so `init()` measures it: two independent windows
//!   of PIT count-down (linear mode 2) versus TSC delta must agree within
//!   [`CAL_AGREE_PERCENT`] and land inside plausible bounds, otherwise boot
//!   fails loudly rather than running on a made-up clock.
//!
//! The clock API ([`now_ticks`], [`ticks_to_us`], [`now_us`]) is the
//! project's monotonic time source from here on: never wall-clock, never
//! re-reads the PIT (that is reserved for future re-calibration/sanity),
//! and honest about its resolution — microseconds derived from a measured
//! integer TSC frequency.
//!
//! Boot-order contract: `init()` runs *after* the M1 suite (M1's regression
//! tests observe firmware-owned hardware state; we must not perturb it
//! first) and *before* the M2 suite (which verifies the calibration
//! independently and consumes the clock).

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::read_tsc;
use crate::drivers::pit;
use crate::log::log_info as info;

/// Kernel tick rate the PIT is left running at after calibration. 100 Hz
/// (10 ms) is the classic scheduler-tick granularity; M3's scheduler
/// consumes these interrupts, nothing before it needs finer.
pub const KERNEL_TICK_HZ: u32 = 100;

/// Each calibration window: ~20 ms of PIT oscillator ticks
/// (1 193 182 Hz / 50). Long enough that latch/poll latency (µs) is deep
/// in the noise, short enough to keep boot snappy under TCG.
const CAL_WINDOW_COUNTS: u32 = pit::OSCILLATOR_HZ as u32 / 50;

/// The two windows must agree on TSC frequency within this many percent.
const CAL_AGREE_PERCENT: u64 = 5;

/// Plausible TSC bounds (Hz). Anything outside means the measurement — not
/// the machine — is broken (QEMU TCG presents a fixed-rate TSC; real hosts
/// are 0.5–6 GHz class). Bounds are deliberately wide.
const TSC_HZ_MIN: u64 = 100_000_000;
const TSC_HZ_MAX: u64 = 8_000_000_000;

/// Calibrated TSC frequency in Hz; 0 until `init()` succeeds.
static TSC_HZ: AtomicU64 = AtomicU64::new(0);

/// Calibrate the TSC against the PIT and start the kernel tick.
///
/// Returns Err (with the PIT left in periodic mode anyway — a ticking
/// timer beats a stopped one for every later consumer) when the measured
/// frequency is implausible or the windows disagree.
pub fn init() -> Result<(), &'static str> {
    // Window 1 + window 2, both on the linear mode-2 countdown. IF is 0
    // here (boot invariant outside the tick tests), so nothing interleaves
    // with the paired (count, TSC) samples.
    // SAFETY: we own the PIT from this point (post-M1 suite, single CPU,
    // IF=0); reprogramming channel 0 is the intended effect.
    unsafe { pit::set_calibration_mode() };
    let (counts1, tsc1) = unsafe { pit::calibrate_tsc_window(CAL_WINDOW_COUNTS) };
    let (counts2, tsc2) = unsafe { pit::calibrate_tsc_window(CAL_WINDOW_COUNTS) };

    // Leave the machine ticking whatever the verdict below is.
    // SAFETY: as above.
    unsafe { pit::set_periodic_hz(KERNEL_TICK_HZ) };

    if counts1 == 0 || counts2 == 0 || tsc1 == 0 || tsc2 == 0 {
        return Err("calibration window measured zero (PIT or TSC not advancing?)");
    }
    let hz1 = tsc1 * pit::OSCILLATOR_HZ / u64::from(counts1);
    let hz2 = tsc2 * pit::OSCILLATOR_HZ / u64::from(counts2);
    let (lo, hi) = if hz1 <= hz2 { (hz1, hz2) } else { (hz2, hz1) };
    info!(
        "timekeeping",
        "tsc calibration: window1 {counts1} counts/{tsc1} tsc = {hz1} Hz, window2 {counts2}/{tsc2} = {hz2} Hz"
    );
    if hi - lo > lo * CAL_AGREE_PERCENT / 100 {
        return Err("calibration windows disagree beyond tolerance");
    }
    let hz = (hz1 + hz2) / 2;
    if !(TSC_HZ_MIN..=TSC_HZ_MAX).contains(&hz) {
        return Err("calibrated TSC frequency outside plausible bounds");
    }
    TSC_HZ.store(hz, Ordering::Relaxed);
    info!(
        "timekeeping",
        "tsc_hz={hz} ({} MHz); PIT ch0 ticking at {KERNEL_TICK_HZ} Hz (mode 3, vector 32 via IOAPIC/LAPIC)",
        hz / 1_000_000
    );
    Ok(())
}

/// Calibrated TSC frequency (Hz); 0 before `init()` succeeds.
pub fn tsc_hz() -> u64 {
    TSC_HZ.load(Ordering::Relaxed)
}

/// Current monotonic clock reading in raw TSC ticks.
pub fn now_ticks() -> u64 {
    read_tsc()
}

/// Convert a TSC tick delta to microseconds using the calibrated frequency.
/// Returns 0 before calibration (callers treat an uncalibrated clock as an
/// error, not as "time 0" — see `init()`'s loud-failure contract).
pub fn ticks_to_us(ticks: u64) -> u64 {
    let hz = tsc_hz();
    if hz == 0 {
        return 0;
    }
    // ticks ≤ ~2^40 in any sane window, hz ≤ 2^33 → product fits u128;
    // u128 division goes through compiler_builtins (proven by M1 wide_math).
    (u128::from(ticks) * 1_000_000 / u128::from(hz)) as u64
}

/// Monotonic microseconds since an arbitrary fixed point (boot). Only
/// deltas are meaningful — there is no epoch.
pub fn now_us() -> u64 {
    ticks_to_us(now_ticks())
}

/// Spin (with PAUSE) for at least `us` microseconds of monotonic time.
/// Used by tests; production code will get proper sleeping in M3 with the
/// scheduler. Bounded by 4× the request as an anti-hang net: returns false
/// if the clock never reached the target (should be impossible post-init).
pub fn busy_wait_us(us: u64) -> bool {
    let start = now_us();
    let mut guard = 0u64;
    while now_us() - start < us {
        core::hint::spin_loop();
        guard += 1;
        if guard >= 2_000_000_000 {
            return false;
        }
    }
    true
}
