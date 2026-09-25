//! Kernel address space (M2.4) — 4-level x86-64 paging, ADR-0008.
//!
//! Layout (all addresses canonical):
//!
//! ```text
//! 0x0000_0000_0000_0000 .. 0x0000_0000_FFFF_FFFF   identity view
//!     Transient compatibility mapping of every firmware-described region.
//!     Firmware code still runs under our tables pre-ExitBootServices
//!     (boot-service calls, the final ResetSystem), so this view exists
//!     until M2.7 tears it down and the kernel owns the machine.
//! 0xFFFF_FFFF_8000_0000 .. 0xFFFF_FFFF_FFFF_FFFF   kernel direct map
//!     VA = phys + KERNEL_OFFSET for the first 2 GiB of described memory.
//!     This is the kernel's permanent view; the boot image executes from
//!     its identity addresses today and from this alias after M2.7.
//! ```
//!
//! Permissions (W^X where *we* own the memory):
//! * our image window, both views, per PE section (via the Loaded Image
//!   Protocol + our own PE headers): `.text` → R+X, `.rdata` → RO+NX,
//!   everything else (`.data`, `.bss`, gaps) → RW+NX. With CR0.WP set,
//!   even ring 0 faults writing RO pages — the M2 tests demand exactly
//!   that, plus NX faulting on execute-from-data (direct-map view).
//! * identity view outside the image window: RW+X, mirroring the
//!   firmware's own permissiveness. This is a deliberate, documented
//!   pre-ExitBootServices carve-out (ADR-0008): firmware executes from
//!   memory the map labels Reserved or BootServicesData (EDK2's
//!   decompressed DXE volumes), so type-based NX in this view faulted
//!   live firmware code on first boot. We own nothing here yet; the view
//!   dies at M2.7 and the direct map enforces NX from day one.
//! * The LAPIC MMIO page is NOT in the UEFI memory map (it is GCD MMIO,
//!   not memory) — it is mapped explicitly from IA32_APIC_BASE, because
//!   the interrupt path EOIs through it.
//! * 2 MiB pages for bulk ranges; the image window is split to 4 KiB
//!   (section boundaries are page-granular in PE images).
//!
//! Deliberate M2 limits (ADR-0008 "disadvantages accepted"): WB cache
//! attributes everywhere (MMIO devices we touch are port-I/O; UC/WC via PAT
//! arrives with real drivers), no global pages, no 1 GiB pages, described
//! regions above 4 GiB are skipped (counted and logged), and the identity
//! view keeps firmware alive rather than being minimal.

use super::{cr0, efer, read_cr0, read_cr3, read_efer, write_cr0, write_cr3, write_efer};
use crate::bootinfo;
use crate::frames;
use crate::log::log_info as info;
use crate::uefi;

/// VA = phys + KERNEL_OFFSET in the kernel direct map (top 2 GiB).
pub const KERNEL_OFFSET: u64 = 0xFFFF_FFFF_8000_0000;

/// Physical span covered by the direct map (from phys 0).
pub const DIRECT_MAP_BYTES: u64 = 2 << 30;

// PTE/PDE flag bits (SDM Vol. 3 §4.5, Table 4-18 for 4-level):
pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITE: u64 = 1 << 1;
/// Bit 63 — honored because EFER.NXE=1 (set in `init()`).
pub const PTE_NX: u64 = 1 << 63;
/// Page Size bit in PD/PDPT entries (2 MiB / 1 GiB page).
const PTE_HUGE: u64 = 1 << 7;
/// Address mask for table/leaf entries (bits 12..51).
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

const PAGE: u64 = 4096;
const PAGE_2M: u64 = 2 << 20;

#[repr(C, align(4096))]
struct PageTable([u64; 512]);

/// Our live PML4 physical address (CR3 value); 0 before `init()`.
static PML4_PHYS: crate::sync::SyncCell<u64> = crate::sync::SyncCell::new(0);
/// Described regions above 4 GiB skipped by the identity/direct-map build
/// (counted for the test's loud reporting; zero on the reference VM's RAM).
static SKIPPED_HIGH_REGIONS: crate::sync::SyncCell<usize> = crate::sync::SyncCell::new(0);

/// PE section permission classes for the image window.
#[derive(Clone, Copy, PartialEq)]
enum Perm {
    /// Read + execute (`.text`).
    Rx,
    /// Read-only (`.rdata`).
    Ro,
    /// Read-write, no-execute (default: `.data`, `.bss`, gaps).
    Rw,
    /// Read-write + execute — the identity view outside our image window
    /// (documented pre-EBS firmware carve-out, module header).
    FwCompat,
}

fn flags_for(perm: Perm) -> u64 {
    PTE_PRESENT
        | match perm {
            Perm::Rx => 0,                  // RO+X: no WRITE, no NX
            Perm::Ro => PTE_NX,             // RO+NX
            Perm::Rw => PTE_WRITE | PTE_NX, // RW+NX
            Perm::FwCompat => PTE_WRITE,    // RW+X (firmware carve-out)
        }
}

/// Image section window: (phys start 4K-aligned, phys end 4K-aligned, perm).
const MAX_SECTIONS: usize = 12;

// SAFETY CONTRACT for this module's statics: as `SyncCell` — single-CPU,
// serialized boot context; `init()` runs exactly once.

/// Build the kernel address space and switch to it.
///
/// `image_handle` is the handle firmware passed to `efi_main`; it yields the
/// loaded-image base/size, whose PE section table yields per-section
/// permissions. `summary` must be from the most recent `bootinfo::capture()`.
///
/// After success, CR3 points at our tables; the identity view keeps every
/// currently-executing address valid, so the caller simply continues.
///
/// # Safety
/// Ring 0, IF=0, frame allocator initialized, memory map freshly captured.
pub unsafe fn init(
    image_handle: usize,
    summary: &bootinfo::MemoryMapSummary,
) -> Result<(), &'static str> {
    let (img_base, img_size) = uefi::loaded_image_info(image_handle)
        .ok_or("Loaded Image Protocol did not report our image")?;
    let img_end = img_base + img_size;

    // PE section table → permission windows inside the image.
    let mut sections: [(u64, u64, Perm); MAX_SECTIONS] = [(0, 0, Perm::Rw); MAX_SECTIONS];
    // SAFETY: `loaded_image_info` returned a plausible image range and the
    // firmware tables (still active) map it readably; parse only reads
    // inside [img_base, img_end) behind bounds checks.
    let nsec = unsafe { parse_pe_sections(img_base, img_end, &mut sections)? };

    let pml4 = new_table().ok_or("out of frames for page tables")?;

    // Both views, built from the same region walk.
    // SAFETY: summary describes the live captured map; region bounds drive
    // only table writes into frames we own; IF=0, single CPU.
    unsafe {
        *SKIPPED_HIGH_REGIONS.get() = 0;
        bootinfo::for_each_region(summary, |d| {
            let base = d.physical_start;
            let size = d.number_of_pages * PAGE;
            if size == 0 {
                return;
            }
            if base > u64::from(u32::MAX) {
                // Above 4 GiB: skipped at M2 (module header), counted loudly.
                *SKIPPED_HIGH_REGIONS.get() += 1;
                return;
            }
            let end = (base + size).min(u64::from(u32::MAX) + 1);
            // Identity view: RW+X like firmware's own tables (module
            // header carve-out); our image window overrides below.
            map_range_2m(pml4, base, base, end - base, flags_for(Perm::FwCompat));
            // Direct-map view: first DIRECT_MAP_BYTES of physical memory,
            // bulk RW+NX; the image window is re-mapped per-section below
            // (window pages must win, so bulk goes first and splits after).
            if base < DIRECT_MAP_BYTES {
                let mend = end.min(DIRECT_MAP_BYTES);
                map_range_2m(
                    pml4,
                    base + KERNEL_OFFSET,
                    base,
                    mend - base,
                    flags_for(Perm::Rw),
                );
            }
        });

        // LAPIC MMIO: absent from the memory map (GCD MMIO, not memory),
        // mandatory for the EOI path. Located from IA32_APIC_BASE, not
        // assumed. (The IOAPIC at 0xFEC00000 IS map-described.)
        const APIC_BASE_MSR: u32 = 0x1B;
        // (RDMSR of the architectural APIC base register — the enclosing
        // block's contract covers ring-0 MSR reads.)
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdmsr", out("eax") lo, out("edx") hi, in("ecx") APIC_BASE_MSR,
            options(nostack, preserves_flags));
        let apic_base = ((hi as u64) << 32 | lo as u64) & 0xF_FFFF_F000;
        if apic_base != 0 {
            map_2m(
                pml4,
                apic_base / PAGE_2M * PAGE_2M,
                apic_base / PAGE_2M * PAGE_2M,
                flags_for(Perm::Rw),
            );
        }

        // Image window last, at 4 KiB granularity, in BOTH views: it must
        // win over (split) any 2 MiB bulk entry covering it.
        let win_start = img_base / PAGE * PAGE;
        let win_end = img_end.div_ceil(PAGE) * PAGE;
        for view in 0..2u64 {
            let off = if view == 0 { 0 } else { KERNEL_OFFSET };
            let mut va = win_start + off;
            while va < win_end + off {
                let phys = va - off;
                let perm = section_perm(phys, &sections, nsec);
                map_page_4k(pml4, va, phys, flags_for(perm));
                va += PAGE;
            }
        }
    }

    // Enable the enforcement bits BEFORE the switch, then switch and verify.
    // SAFETY: EFER gains NXE (firmware already set it on this platform —
    // asserting, not assuming); CR0 gains WP so ring-0 writes honor RO.
    unsafe {
        let efer_v = read_efer();
        if efer_v & efer::NXE == 0 {
            write_efer(efer_v | efer::NXE);
        }
        let cr0_v = read_cr0();
        if cr0_v & cr0::WP == 0 {
            write_cr0(cr0_v | cr0::WP);
        }
        write_cr3(pml4 as u64);
        if read_cr3() & ADDR_MASK != pml4 as u64 {
            return Err("CR3 read-back does not match our PML4");
        }
        *PML4_PHYS.get() = pml4 as u64;
    }

    let skipped = unsafe { *SKIPPED_HIGH_REGIONS.get() };
    info!(
        "paging",
        "address space live: cr3={:#x} identity+direct-map(offset {KERNEL_OFFSET:#x}), image window [{img_base:#x},{img_end:#x}) {nsec} sections, skipped_high_regions={skipped}",
        pml4 as u64
    );
    Ok(())
}

/// CR3 value installed by `init()` (0 before). Test evidence: must equal
/// the live CR3 read-back.
pub fn cr3_phys() -> u64 {
    // SAFETY: plain read of a boot-initialized cell (module contract).
    unsafe { *PML4_PHYS.get() }
}

/// Translate a direct-map VA back to physical (None if outside the window).
pub fn direct_map_to_phys(va: u64) -> Option<u64> {
    if va >= KERNEL_OFFSET && va - KERNEL_OFFSET < DIRECT_MAP_BYTES {
        Some(va - KERNEL_OFFSET)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Table construction
// ---------------------------------------------------------------------------

/// Allocate and zero one 4 KiB table frame.
fn new_table() -> Option<*mut PageTable> {
    let f = frames::alloc()?;
    let p = f as *mut PageTable;
    // SAFETY: freshly allocated exclusive frame, identity-mapped by the
    // firmware tables we are still running under (init runs pre-switch).
    unsafe { (*p).0 = [0u64; 512] };
    Some(p)
}

/// Walk (creating as needed) to the next-level table at `table[idx]`.
/// Entries here are always table pointers (huge pages are split before
/// descending — see `map_page_4k`).
///
/// # Safety
/// `table` is a live, owned, identity-accessible page table frame.
unsafe fn next_level(table: *mut PageTable, idx: usize) -> Option<*mut PageTable> {
    unsafe {
        let e = (*table).0[idx];
        if e & PTE_PRESENT != 0 {
            return Some((e & ADDR_MASK) as *mut PageTable);
        }
        let child = new_table()?;
        (*table).0[idx] = child as u64 | PTE_PRESENT | PTE_WRITE;
        Some(child)
    }
}

const fn pml4_idx(va: u64) -> usize {
    ((va >> 39) & 0x1FF) as usize
}
const fn pdpt_idx(va: u64) -> usize {
    ((va >> 30) & 0x1FF) as usize
}
const fn pd_idx(va: u64) -> usize {
    ((va >> 21) & 0x1FF) as usize
}
const fn pt_idx(va: u64) -> usize {
    ((va >> 12) & 0x1FF) as usize
}

/// Map `[va, va+size)` → `[pa, pa+size)` with 2 MiB pages. `va`, `pa`, and
/// `size` must be 2 MiB aligned/multiples; callers round *regions* outward
/// via `map_range_2m`, which clips to the 2 MiB grid (partial edges get
/// whole 2 MiB pages — over-mapping ≤2 MiB per edge into neighbors of the
/// same described region set; documented in ADR-0008).
///
/// # Safety
/// Tables and target frames are owned/identity-accessible; IF=0.
unsafe fn map_2m(pml4: *mut PageTable, va: u64, pa: u64, flags: u64) {
    unsafe {
        let pdpt = next_level(pml4, pml4_idx(va)).expect("frames exhausted (bulk map)");
        let pd = next_level(pdpt, pdpt_idx(va)).expect("frames exhausted (bulk map)");
        (*pd).0[pd_idx(va)] = (pa & ADDR_MASK) | flags | PTE_HUGE;
    }
}

/// Map one 4 KiB page, splitting any 2 MiB huge page that currently covers
/// it (the split preserves the huge page's flags for its other 511 pages).
///
/// # Safety
/// As `map_2m`.
unsafe fn map_page_4k(pml4: *mut PageTable, va: u64, pa: u64, flags: u64) {
    unsafe {
        let pdpt = next_level(pml4, pml4_idx(va)).expect("frames exhausted (window map)");
        let pd = next_level(pdpt, pdpt_idx(va)).expect("frames exhausted (window map)");
        let pde = (*pd).0[pd_idx(va)];
        let pt;
        if pde & PTE_PRESENT != 0 && pde & PTE_HUGE != 0 {
            // Split: build a PT replicating the huge page's mapping, then
            // replace the PDE with a pointer to it.
            let new_pt = new_table().expect("frames exhausted (huge split)");
            let huge_pa = pde & ADDR_MASK;
            let huge_flags = pde & !ADDR_MASK & !PTE_HUGE;
            for i in 0..512 {
                (*new_pt).0[i] = (huge_pa + (i as u64) * PAGE) | huge_flags;
            }
            (*pd).0[pd_idx(va)] = new_pt as u64 | PTE_PRESENT | PTE_WRITE;
            pt = new_pt;
        } else {
            pt = next_level(pd, pd_idx(va)).expect("frames exhausted (window map)");
        }
        (*pt).0[pt_idx(va)] = (pa & ADDR_MASK) | flags;
    }
}

/// Bulk-map a region span with 2 MiB pages, clipping/rounding to the grid.
///
/// # Safety
/// As `map_2m`.
unsafe fn map_range_2m(pml4: *mut PageTable, va: u64, pa: u64, size: u64, flags: u64) {
    unsafe {
        // Callers pass va = pa + delta with delta ∈ {0, KERNEL_OFFSET},
        // both 2 MiB multiples, so grid-aligned blocks keep both sides
        // aligned.
        let delta = va - pa;
        let blk_start = pa / PAGE_2M * PAGE_2M;
        let blk_end = (pa + size).div_ceil(PAGE_2M) * PAGE_2M;
        let mut p = blk_start;
        while p < blk_end {
            map_2m(pml4, p + delta, p, flags);
            p += PAGE_2M;
        }
    }
}

// ---------------------------------------------------------------------------
// PE section parsing (our own image)
// ---------------------------------------------------------------------------

/// Parse our own PE headers at `img_base`: fill `out` with up to
/// `MAX_SECTIONS` (start, end, perm) windows; return the count.
///
/// # Safety
/// `img_base..img_end` must be our loaded, readable PE image.
unsafe fn parse_pe_sections(
    img_base: u64,
    img_end: u64,
    out: &mut [(u64, u64, Perm); MAX_SECTIONS],
) -> Result<usize, &'static str> {
    // SAFETY: caller guarantees the image range is readable; all reads are
    // bounded-checks-then-read against img_end; no writes.
    unsafe {
        let read_u16 = |off: u64| -> Option<u16> {
            if img_base + off + 2 <= img_end {
                Some(((img_base + off) as *const u16).read_unaligned())
            } else {
                None
            }
        };
        let read_u32 = |off: u64| -> Option<u32> {
            if img_base + off + 4 <= img_end {
                Some(((img_base + off) as *const u32).read_unaligned())
            } else {
                None
            }
        };
        if read_u16(0).ok_or("truncated image: no DOS header")? != 0x5A4D {
            return Err("image base does not start with MZ");
        }
        let pe_off = read_u32(0x3C).ok_or("truncated DOS header")? as u64;
        if read_u32(pe_off).ok_or("truncated PE header")? != 0x0000_4550 {
            return Err("missing PE signature");
        }
        let coff = pe_off + 4;
        let machine = read_u16(coff).ok_or("truncated COFF header")?;
        if machine != 0x8664 {
            return Err("not an x86-64 PE image");
        }
        let nsec = read_u16(coff + 2).ok_or("truncated COFF header")? as usize;
        let opt_size = read_u16(coff + 16).ok_or("truncated COFF header")? as u64;
        let sec_table = coff + 20 + opt_size;
        let mut n = 0usize;
        for i in 0..nsec.min(MAX_SECTIONS) {
            let sh = sec_table + (i as u64) * 40;
            let name_ptr = (img_base + sh) as *const u8;
            if img_base + sh + 40 > img_end {
                break;
            }
            let mut name = [0u8; 8];
            core::ptr::copy_nonoverlapping(name_ptr, name.as_mut_ptr(), 8);
            let vsize = read_u32(sh + 8).unwrap_or(0) as u64;
            let rva = read_u32(sh + 12).unwrap_or(0) as u64;
            if vsize == 0 {
                continue;
            }
            let start = img_base + rva;
            let end = start + vsize;
            let perm = match &name {
                b".text\0\0\0" => Perm::Rx,
                b".rdata\0\0" => Perm::Ro,
                _ => Perm::Rw,
            };
            out[n] = (start / PAGE * PAGE, end.div_ceil(PAGE) * PAGE, perm);
            n += 1;
        }
        if n == 0 {
            return Err("PE image reported no usable sections");
        }
        Ok(n)
    }
}

/// Permission for one physical page inside the image window: the matching
/// section's class, RW+NX for header/gap pages.
fn section_perm(phys: u64, sections: &[(u64, u64, Perm); MAX_SECTIONS], n: usize) -> Perm {
    for (start, end, perm) in sections.iter().take(n) {
        if phys >= *start && phys < *end {
            return *perm;
        }
    }
    Perm::Rw
}
