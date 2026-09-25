//! Boot → kernel handoff record (M2.7 ABI boundary, ADR-0011).
//!
//! Plain data only: the boot stage (a UEFI application) fills this record,
//! the kernel proper consumes it — no firmware *types* ever cross the
//! boundary. Two deliberate exceptions, both documented ABI:
//!
//! * Region `kind` values are the UEFI spec's numeric memory types (the
//!   constants below). They are facts about firmware memory the same way
//!   an ACPI table is: the kernel must interpret them, but it does not
//!   need UEFI to do so.
//! * The `ResetSystem` runtime-service entry point is handed to
//!   [`crate::halt`] as a raw `efiapi` function pointer — runtime
//!   services survive `ExitBootServices` by design (UEFI 2.10 §8.6), and
//!   it is the one firmware function the kernel proper may call.
//!
//! Filling happens once per boot, sequentially, IF=0 (boot contract), via
//! `begin_map` + `push_region` + `set_image_layout`; reads are plain
//! copy-outs. `ExitBootServices` may change the map, so the boot stage
//! re-fills the record with the *final* map at exit time (M2.7).

use crate::sync::SyncCell;

// UEFI numeric memory types (UEFI 2.10 §7.2, Table 31) — handoff ABI.
pub const KIND_RESERVED: u32 = 0;
pub const KIND_LOADER_CODE: u32 = 1;
pub const KIND_LOADER_DATA: u32 = 2;
pub const KIND_BOOT_SERVICES_CODE: u32 = 3;
pub const KIND_BOOT_SERVICES_DATA: u32 = 4;
pub const KIND_RUNTIME_SERVICES_CODE: u32 = 5;
pub const KIND_RUNTIME_SERVICES_DATA: u32 = 6;
pub const KIND_CONVENTIONAL: u32 = 7;
pub const KIND_UNUSABLE: u32 = 8;
pub const KIND_ACPI_RECLAIM: u32 = 9;
pub const KIND_ACPI_NVS: u32 = 10;
pub const KIND_MMIO: u32 = 11;
pub const KIND_MMIO_PORT_SPACE: u32 = 12;

/// One memory-map region as described by firmware at fill time.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    /// UEFI numeric memory type (`KIND_*`).
    pub kind: u32,
    pub base: u64,
    pub pages: u64,
    /// UEFI attribute bits (EFI_MEMORY_UC/WP/WT/RP/XP/RO/NV per spec §7.2).
    pub attribute: u64,
}

/// Our own loaded image as firmware reported it (Loaded Image Protocol):
/// paging turns the PE section table inside this range into the
/// per-section permission window (.text R+X, .rdata RO, rest RW+NX).
#[derive(Clone, Copy, Debug)]
pub struct ImageLayout {
    pub base: u64,
    pub size: u64,
}

/// OVMF at 512M reports a couple dozen regions; 256 is ample headroom.
/// Overflow never corrupts: extra regions are counted in
/// [`region_overflow()`], and the test suite asserts it stays 0.
const REGION_CAP: usize = 256;

const EMPTY_REGION: Region = Region {
    kind: 0,
    base: 0,
    pages: 0,
    attribute: 0,
};

// SAFETY CONTRACT for the statics below: as `SyncCell` — single-CPU,
// interrupts-off boot context; the boot stage is the only writer,
// sequentially, before kernel entry.
static REGIONS: SyncCell<[Region; REGION_CAP]> = SyncCell::new([EMPTY_REGION; REGION_CAP]);
static COUNT: SyncCell<usize> = SyncCell::new(0);
static OVERFLOW: SyncCell<usize> = SyncCell::new(0);
static MAP_KEY: SyncCell<u64> = SyncCell::new(0);
static IMAGE: SyncCell<ImageLayout> = SyncCell::new(ImageLayout { base: 0, size: 0 });

/// Reset the region list and record the map key that belongs to it. The
/// boot stage calls this before re-pushing regions (initial capture *and*
/// the final pre-`ExitBootServices` capture).
pub fn begin_map(map_key: u64) {
    // SAFETY: boot contract — single sequential writer, IF=0.
    unsafe {
        *COUNT.get() = 0;
        *OVERFLOW.get() = 0;
        *MAP_KEY.get() = map_key;
    }
}

/// Append one region. Beyond [`REGION_CAP`] the region is dropped and
/// counted in [`region_overflow()`] (tests assert that never happens).
pub fn push_region(region: Region) {
    // SAFETY: boot contract, as `begin_map`.
    unsafe {
        let n = *COUNT.get();
        if n < REGION_CAP {
            (*REGIONS.get())[n] = region;
            *COUNT.get() = n + 1;
        } else {
            *OVERFLOW.get() += 1;
        }
    }
}

/// Record the loaded-image layout (base/size from firmware).
pub fn set_image_layout(layout: ImageLayout) {
    // SAFETY: boot contract, as `begin_map`.
    unsafe { *IMAGE.get() = layout };
}

/// Map key of the last filled map (replayed to `ExitBootServices`).
pub fn map_key() -> u64 {
    // SAFETY: boot contract; copy-out.
    unsafe { *MAP_KEY.get() }
}

/// Number of regions in the record.
pub fn region_count() -> usize {
    // SAFETY: boot contract; copy-out.
    unsafe { *COUNT.get() }
}

/// Regions dropped for exceeding [`REGION_CAP`] during the last fill.
pub fn region_overflow() -> usize {
    // SAFETY: boot contract; copy-out.
    unsafe { *OVERFLOW.get() }
}

/// Region `i`, or `None` past [`region_count()`].
pub fn region(i: usize) -> Option<Region> {
    // SAFETY: boot contract; bounds-checked copy-out.
    unsafe {
        if i < *COUNT.get() {
            Some((*REGIONS.get())[i])
        } else {
            None
        }
    }
}

/// The loaded-image layout, or `None` if it was never recorded.
pub fn image_layout() -> Option<ImageLayout> {
    // SAFETY: boot contract; copy-out.
    unsafe {
        let l = *IMAGE.get();
        if l.base != 0 && l.size != 0 {
            Some(l)
        } else {
            None
        }
    }
}
