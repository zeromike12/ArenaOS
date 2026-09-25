//! Boot information capture: the UEFI memory map.
//!
//! This module is the seed of the **boot-info record** contract (ADR-0003):
//! the arch-neutral hand-off between boot stage and kernel proper. At M1 it
//! captures, classifies, and *verifies* the firmware memory map; at M2 the
//! frame allocator is built from exactly this data and `map_key` is replayed
//! into `ExitBootServices()`.
//!
//! Reference: UEFI 2.10 §7.2.10 (GetMemoryMap).

use core::cell::UnsafeCell;

use crate::uefi::{self, EFI_BUFFER_TOO_SMALL, EFI_SUCCESS, MemoryDescriptor, Status};

/// Status sentinel for "boot stage invoked without a captured system table"
/// (a programming error; the M1 test would report it as a capture failure).
pub const ERR_NO_FIRMWARE: Status = usize::MAX;

/// Static capture buffer. 32 KiB holds ~680 descriptors — QEMU/OVMF at 512M
/// typically reports a few dozen. If firmware ever needs more, `capture()`
/// reports EFI_BUFFER_TOO_SMALL honestly instead of overflowing.
const MAP_BUF_BYTES: usize = 32 * 1024;

/// 8-byte aligned raw buffer (descriptors require UINT64 alignment).
/// SAFETY CONTRACT: single-CPU, interrupts-off boot context (UEFI mandate);
/// `capture()` runs exactly once, sequentially, before any concurrency.
#[repr(align(8))]
struct AlignedBuf(UnsafeCell<[u8; MAP_BUF_BYTES]>);
unsafe impl Sync for AlignedBuf {}

impl AlignedBuf {
    /// Raw pointer to the buffer. SAFETY CONTRACT as on `AlignedBuf`:
    /// single-writer sequential boot-time use only.
    const fn get(&self) -> *mut [u8; MAP_BUF_BYTES] {
        self.0.get()
    }
}

static MAP_BUF: AlignedBuf = AlignedBuf(UnsafeCell::new([0u8; MAP_BUF_BYTES]));

/// Classified summary of the firmware memory map.
#[derive(Clone, Copy, Debug)]
pub struct MemoryMapSummary {
    pub region_count: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
    /// Firmware map key — replayed to ExitBootServices (M2).
    pub map_key: usize,
    /// EfiConventionalMemory: free, usable for the frame allocator.
    pub conventional_pages: u64,
    /// Boot-services code/data + ACPI reclaim: freed after
    /// ExitBootServices (reclaimed in M2).
    pub reclaimable_pages: u64,
    /// Runtime services code/data + ACPI NVS: never touch, ever.
    pub runtime_pages: u64,
    /// Reserved/unusable/MMIO/loader/pal/persistent: not allocatable.
    pub reserved_pages: u64,
    /// Largest single conventional region — best candidate for early kernel
    /// allocations in M2.
    pub largest_conventional_base: u64,
    pub largest_conventional_pages: u64,
}

impl MemoryMapSummary {
    pub const PAGE: u64 = 4096;
    pub fn conventional_mib(&self) -> u64 {
        self.conventional_pages * Self::PAGE / (1024 * 1024)
    }
    pub fn reclaimable_mib(&self) -> u64 {
        self.reclaimable_pages * Self::PAGE / (1024 * 1024)
    }
    pub fn largest_conventional_mib(&self) -> u64 {
        self.largest_conventional_pages * Self::PAGE / (1024 * 1024)
    }
    /// Sum of ALL described address spans — conventional, reclaimable,
    /// runtime, and reserved/MMIO. Not installed RAM: firmware maps huge
    /// reserved device windows (e.g. the 64-bit PCI MMIO hole) into the
    /// memory map, so this legitimately exceeds physical memory. Useful as
    /// a map-completeness cross-check, not as a RAM size.
    pub fn total_mib(&self) -> u64 {
        (self.conventional_pages
            + self.reclaimable_pages
            + self.runtime_pages
            + self.reserved_pages)
            * Self::PAGE
            / (1024 * 1024)
    }
}

/// Call UEFI GetMemoryMap into the static buffer and classify it.
pub fn capture() -> Result<MemoryMapSummary, Status> {
    // SAFETY: single-CPU boot context (buffer contract above); the buffer is
    // 8-aligned inside our identity-mapped image. `boot_services()` is valid
    // pre-ExitBootServices, which is exactly when this runs.
    unsafe {
        let Some(boot) = uefi::boot_services() else {
            return Err(ERR_NO_FIRMWARE);
        };
        let buf = MAP_BUF.get();
        let mut map_size = MAP_BUF_BYTES;
        let mut map_key = 0usize;
        let mut desc_size = 0usize;
        let mut desc_version = 0u32;

        let mut status = (boot.get_memory_map)(
            &mut map_size,
            buf.cast::<u8>(),
            &mut map_key,
            &mut desc_size,
            &mut desc_version,
        );

        if status == EFI_BUFFER_TOO_SMALL && map_size <= MAP_BUF_BYTES {
            // Firmware updated map_size to what it wants; retry once.
            status = (boot.get_memory_map)(
                &mut map_size,
                buf.cast::<u8>(),
                &mut map_key,
                &mut desc_size,
                &mut desc_version,
            );
        }
        // EDK2 restores TPL_APPLICATION on the way out of every boot
        // service, which means `sti`. M1 invariant: IF=0 in our code; our
        // IDT absorbs whatever fires during the call itself.
        crate::arch::x86_64::cli();

        if status != EFI_SUCCESS {
            return Err(status);
        }

        Ok(classify(buf, map_size, map_key, desc_size, desc_version))
    }
}

/// Walk the raw descriptor array. `desc_size` comes from firmware (the spec
/// allows padding beyond sizeof(EFI_MEMORY_DESCRIPTOR) for future versions),
/// so we stride by it, not by our struct size.
///
/// # Safety
/// `buf` must hold `map_size` valid bytes of version-1 descriptors at
/// `desc_size` stride (guaranteed by a successful GetMemoryMap call).
unsafe fn classify(
    buf: *const [u8; MAP_BUF_BYTES],
    map_size: usize,
    map_key: usize,
    desc_size: usize,
    desc_version: u32,
) -> MemoryMapSummary {
    use crate::uefi::memory_type::*;

    let mut s = MemoryMapSummary {
        region_count: 0,
        descriptor_size: desc_size,
        descriptor_version: desc_version,
        map_key,
        conventional_pages: 0,
        reclaimable_pages: 0,
        runtime_pages: 0,
        reserved_pages: 0,
        largest_conventional_base: 0,
        largest_conventional_pages: 0,
    };

    if desc_size < core::mem::size_of::<MemoryDescriptor>() {
        return s; // firmware gave nonsense; region_count=0 fails the M1 test
    }

    let count = map_size / desc_size;
    for i in 0..count {
        // SAFETY: stride validated above; caller guarantees `map_size` valid
        // bytes at this stride. read_unaligned tolerates any residual
        // misalignment inside the padded descriptor layout.
        let desc: MemoryDescriptor =
            unsafe { core::ptr::read_unaligned(buf.cast::<u8>().add(i * desc_size).cast()) };
        s.region_count += 1;
        match desc.memory_type {
            CONVENTIONAL => {
                s.conventional_pages += desc.number_of_pages;
                if desc.number_of_pages > s.largest_conventional_pages {
                    s.largest_conventional_pages = desc.number_of_pages;
                    s.largest_conventional_base = desc.physical_start;
                }
            }
            BOOT_SERVICES_CODE | BOOT_SERVICES_DATA | ACPI_RECLAIM => {
                s.reclaimable_pages += desc.number_of_pages;
            }
            RUNTIME_SERVICES_CODE | RUNTIME_SERVICES_DATA | ACPI_NVS => {
                s.runtime_pages += desc.number_of_pages;
            }
            _ => s.reserved_pages += desc.number_of_pages,
        }
    }
    s
}

/// Iterate raw descriptors for human dumping (the M1 test logs a few).
///
/// # Safety
/// Valid only immediately after a successful [`capture()`], with the summary
/// that call returned (buffer intact, pre-ExitBootServices, no concurrency).
pub unsafe fn for_each_region(summary: &MemoryMapSummary, mut f: impl FnMut(&MemoryDescriptor)) {
    if summary.descriptor_size < core::mem::size_of::<MemoryDescriptor>() {
        return;
    }
    // SAFETY: caller contract above — buffer holds region_count descriptors
    // at descriptor_size stride from the successful capture.
    unsafe {
        let buf: *const u8 = MAP_BUF.get().cast::<u8>();
        for i in 0..summary.region_count {
            let desc: MemoryDescriptor =
                core::ptr::read_unaligned(buf.add(i * summary.descriptor_size).cast());
            f(&desc);
        }
    }
}
