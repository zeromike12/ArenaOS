//! Physical frame allocator (M2.3) — flat bitmap over conventional memory.
//!
//! Design (ADR-0007): one bit per 4 KiB physical frame over the span
//! [0, PHYS_LIMIT), 1 = used/reserved. Built from the captured UEFI memory
//! map's *conventional* regions only: everything else (firmware, runtime
//! services, ACPI NVS, MMIO, our own image — which firmware reports as
//! Loader*) never becomes allocatable, so "UEFI runtime regions preserved"
//! is structural, not a filter at alloc time.
//!
//! Scope decisions, all deliberate and documented:
//! * PHYS_LIMIT = 4 GiB caps the bitmap at 128 KiB of static .bss. A 512 MiB
//!   QEMU VM has no conventional memory above 4 GiB; when high memory
//!   matters (bigger targets), this becomes a two-level bitmap or the
//!   allocator moves to dynamic storage — revisit trigger in ADR-0007.
//! * Frames below RESERVE_BELOW (1 MiB) stay reserved: legacy structures
//!   (real-mode IVT area, BDA, VGA hole) cost nothing to protect.
//! * First-fit with a roving hint over 64-bit words: O(1) free, amortized
//!   O(1) alloc in the common case, deterministic, and the measured
//!   throughput (M2 test logs ns/op) is the benchmark behind ADR-0007.
//! * `alloc_contiguous` scans frames linearly — fine for the small runs the
//!   kernel needs near boot (page-table pages arrive in M2.4 one frame at a
//!   time); the heap milestone (2.5) re-evaluates if that changes.
//! * Double free / freeing an unmanaged address is a checked error, not a
//!   silent corruption — the allocator is the one component allowed to be
//!   paranoid about everything above it.
//!
//! The core bit logic is written as pure operations on the bitmap slice so
//! it can move to a host-testable crate wholesale at the M2.7 boot split.

use core::cell::UnsafeCell;

use crate::handoff;
use crate::sync::SyncCell;

pub const FRAME_BYTES: u64 = 4096;

/// Bitmap coverage: [0, 4 GiB). 4 GiB / 4 KiB = 1 Mi frames = 128 KiB of
/// u64 words — static .bss, zero-initialized, never paged out.
pub const PHYS_LIMIT: u64 = 4 << 30;
const FRAME_COUNT: usize = (PHYS_LIMIT / FRAME_BYTES) as usize; // 1 Mi
const WORDS: usize = FRAME_COUNT / 64; // 16384

/// Frames starting here are managed; below stays permanently reserved.
pub const RESERVE_BELOW: u64 = 1 << 20;

/// Bitmap storage. SAFETY CONTRACT: as `SyncCell` — single-CPU, serialized
/// boot context; the M3+ kernel replaces this with a properly synchronized
/// (or per-CPU) allocator; nothing here survives the boot split unchanged.
struct Bitmap(UnsafeCell<[u64; WORDS]>);
unsafe impl Sync for Bitmap {}
static BITMAP: Bitmap = Bitmap(UnsafeCell::new([0u64; WORDS]));

/// Roving first-fit hint (word index).
static HINT: SyncCell<usize> = SyncCell::new(0);
/// Frames under management (managed span size) and currently free.
static TOTAL: SyncCell<u64> = SyncCell::new(0);
static FREE: SyncCell<u64> = SyncCell::new(0);
/// Set after a successful `init()`.
static READY: SyncCell<bool> = SyncCell::new(false);

// SAFETY (all statics above): single-CPU boot stage, IF=0 outside firmware
// calls, no allocation happens across firmware calls — see SyncCell docs.

/// Build the allocator from the captured memory map's conventional regions.
/// Must run after the boot stage filled the handoff record. Fails loudly if nothing usable was
/// captured — a kernel with no memory is not a kernel.
pub fn init() -> Result<(), &'static str> {
    // SAFETY: sequential boot init, single writer (contract above).
    unsafe {
        if *READY.get() {
            return Err("frame allocator initialized twice");
        }
        let map = BITMAP.0.get();
        // Start fully reserved; poke holes where memory is genuinely free.
        (*map).fill(u64::MAX);
        *HINT.get() = 0;
        *TOTAL.get() = 0;
        *FREE.get() = 0;

        let regions = handoff::region_count();
        if regions == 0 {
            return Err("handoff record has no memory regions (boot stage did not fill it?)");
        }
        for i in 0..regions {
            let Some(r) = handoff::region(i) else {
                continue;
            };
            if r.kind != handoff::KIND_CONVENTIONAL {
                continue;
            }
            // Clip the region to the managed span, frame-aligning inward so
            // partial frames at either edge are never handed out.
            let start = r.base.max(RESERVE_BELOW).div_ceil(FRAME_BYTES) * FRAME_BYTES;
            let end = (r.base + r.pages * FRAME_BYTES).min(PHYS_LIMIT) / FRAME_BYTES * FRAME_BYTES;
            if end <= start {
                continue;
            }
            let mut f = start;
            while f < end {
                let idx = (f / FRAME_BYTES) as usize;
                (*map)[idx / 64] &= !(1u64 << (idx % 64));
                *TOTAL.get() += 1;
                *FREE.get() += 1;
                f += FRAME_BYTES;
            }
        }
        if *TOTAL.get() == 0 {
            return Err("conventional regions produced zero manageable frames");
        }
        *READY.get() = true;
        Ok(())
    }
}

/// Allocate one physical frame. Returns its base address (4 KiB aligned by
/// construction), or None when the managed memory is exhausted.
pub fn alloc() -> Option<u64> {
    // SAFETY: single-CPU serialized context (module contract).
    unsafe {
        if !*READY.get() {
            return None;
        }
        let map = BITMAP.0.get();
        let start = *HINT.get();
        let mut w = start;
        loop {
            let inv = !(*map)[w];
            if inv != 0 {
                let b = inv.trailing_zeros() as usize;
                (*map)[w] |= 1u64 << b;
                *HINT.get() = w;
                *FREE.get() -= 1;
                return Some(((w * 64 + b) as u64) * FRAME_BYTES);
            }
            w = (w + 1) % WORDS;
            if w == start {
                return None;
            }
        }
    }
}

/// Free one frame. Errors (without touching state) on misaligned,
/// out-of-span, or not-currently-allocated addresses — double frees are
/// caught, not absorbed.
pub fn free(base: u64) -> Result<(), &'static str> {
    if !base.is_multiple_of(FRAME_BYTES) || base >= PHYS_LIMIT {
        return Err("free: address not a managed frame base");
    }
    // SAFETY: single-CPU serialized context (module contract).
    unsafe {
        if !*READY.get() {
            return Err("free: allocator not initialized");
        }
        let map = BITMAP.0.get();
        let idx = (base / FRAME_BYTES) as usize;
        let bit = 1u64 << (idx % 64);
        if (*map)[idx / 64] & bit == 0 {
            return Err("free: frame was not allocated (double free?)");
        }
        (*map)[idx / 64] &= !bit;
        *FREE.get() += 1;
        // Roving hint follows the earliest frees so first-fit stays cheap.
        if idx / 64 < *HINT.get() {
            *HINT.get() = idx / 64;
        }
        Ok(())
    }
}

/// Allocate `n` physically contiguous frames (n ≥ 1). Linear scan from the
/// hint frame, wrapping once — documented scope note in the module header.
pub fn alloc_contiguous(n: usize) -> Option<u64> {
    // SAFETY: single-CPU serialized context (module contract).
    unsafe {
        if !*READY.get() || n == 0 || n as u64 > total_frames() || n > FRAME_COUNT {
            return None;
        }
        let map = BITMAP.0.get();
        let bit_at = |idx: usize| -> bool { (*map)[idx / 64] & (1u64 << (idx % 64)) != 0 };
        let claim = |i: usize| {
            for k in 0..n {
                let idx = i + k;
                (*map)[idx / 64] |= 1u64 << (idx % 64);
            }
            *FREE.get() -= n as u64;
            *HINT.get() = i / 64;
            (i as u64) * FRAME_BYTES
        };

        let start = (*HINT.get() * 64).min(FRAME_COUNT - n);
        let mut i = start;
        let mut wrapped = false;
        loop {
            // Try a run at i whenever a full run still fits before the end.
            if i + n <= FRAME_COUNT && !bit_at(i) {
                let mut run = 1;
                while run < n && !bit_at(i + run) {
                    run += 1;
                }
                if run == n {
                    return Some(claim(i));
                }
                i += run; // skip the too-short free run
                continue;
            }
            i += 1;
            if i + n > FRAME_COUNT {
                if wrapped {
                    return None;
                }
                wrapped = true;
                i = 0;
            }
            if wrapped && i >= start {
                return None; // scanned [start, end) + [0, start)
            }
        }
    }
}

/// Free a contiguous run previously returned by [`alloc_contiguous`].
pub fn free_contiguous(base: u64, n: usize) -> Result<(), &'static str> {
    for k in 0..n as u64 {
        free(base + k * FRAME_BYTES)?;
    }
    Ok(())
}

/// Frames under management (constant after `init()`).
/// M2.7 post-`ExitBootServices` reconciliation. The allocator was built
/// from an *earlier* capture; firmware may have changed the map between
/// then and the final capture that fed `ExitBootServices` (its own
/// allocations, bookkeeping). Rule: a frame still marked FREE here but
/// not inside a Conventional region of the FINAL handoff map is leaked —
/// marked in-use, never handed out. Frames the final map gained are
/// deliberately NOT claimed (conservative: leaked, not chased). Returns
/// the number of leaked frames (0 means the maps agreed exactly).
pub fn reconcile_final_map() -> Result<u64, &'static str> {
    // SAFETY: boot contract — single CPU, IF=0; sequential post-EBS init.
    unsafe {
        if !*READY.get() {
            return Err("reconcile before init");
        }
        if crate::handoff::region_count() == 0 {
            return Err("handoff record empty at reconcile");
        }
        let map = BITMAP.0.get();
        let mut leaked = 0u64;
        for word_idx in 0..WORDS {
            // Free frames are ZERO bits (bitmap starts all-reserved and
            // init pokes holes); iterate the free ones.
            let mut inv = !(*map)[word_idx];
            while inv != 0 {
                let bit = inv.trailing_zeros() as usize;
                inv &= inv - 1;
                let frame = (word_idx * 64 + bit) as u64 * FRAME_BYTES;
                if !in_final_conventional(frame) {
                    (*map)[word_idx] |= 1u64 << bit;
                    *FREE.get() -= 1;
                    leaked += 1;
                }
            }
        }
        Ok(leaked)
    }
}

/// Is `frame`'s physical address inside a Conventional region of the
/// final handoff map? Linear scan over a couple dozen regions per free
/// frame — a one-time post-EBS pass, simplicity beats cleverness here.
fn in_final_conventional(frame: u64) -> bool {
    for i in 0..crate::handoff::region_count() {
        let Some(r) = crate::handoff::region(i) else {
            continue;
        };
        if r.kind == crate::handoff::KIND_CONVENTIONAL
            && frame >= r.base
            && frame < r.base + r.pages * FRAME_BYTES
        {
            return true;
        }
    }
    false
}

pub fn total_frames() -> u64 {
    // SAFETY: plain read of a boot-initialized counter (contract above).
    unsafe { *TOTAL.get() }
}

/// Frames currently free.
pub fn free_frames() -> u64 {
    // SAFETY: as `total_frames`.
    unsafe { *FREE.get() }
}

/// True after a successful [`init()`].
pub fn ready() -> bool {
    // SAFETY: as `total_frames`.
    unsafe { *READY.get() }
}
