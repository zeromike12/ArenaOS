//! ADR-0095: bounded process-owned native virtual-memory reservations.
//!
//! Reservation IDs are descriptive and never accepted without the exact
//! process-local `VmRegion` capability. Backing pages are private RAM frames
//! allocated only at commit time and returned on explicit release or normal
//! address-space teardown.

use crate::arch::x86_64::paging;
use crate::cap::{Cap, CapObj, RIGHTS_DESTROY, RIGHTS_READ, RIGHTS_WRITE};
use crate::frames;
use crate::sync::{SyncCell, without_interrupts};

pub const MAX_REGIONS: usize = 128;
pub const MAX_REGION_PAGES: u32 = 4096;
pub const MAX_REGIONS_PER_PROCESS: usize = 8;
pub const MAX_COMMITTED_PER_PROCESS: u32 = 8192;
pub const MAX_COMMITTED_PAGES: u32 = 32768;
pub const MAX_OPERATION_PAGES: u32 = 64;
pub const GUARD_PAGES: u64 = 1;

/// This arena is separate from SYS_MAP_MEMORY's 16 TiB mapping window.
const VM_BASE: u64 = 0x0000_2000_0000_0000;
const VM_SLOT_STRIDE: u64 = 18 * 1024 * 1024;
const VM_ARENA_END: u64 = VM_BASE + (MAX_REGIONS as u64 + 1) * VM_SLOT_STRIDE;
const BITMAP_WORDS: usize = MAX_REGION_PAGES as usize / 64;

pub const PROT_READ: u64 = 1;
pub const PROT_WRITE: u64 = 2;
pub const PROT_EXEC: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmError {
    BadArgument,
    Quota,
    Busy,
    BadAddress,
}

#[derive(Clone, Copy)]
struct Region {
    active: bool,
    id: u32,
    pid: u64,
    base: u64,
    pages: u32,
    committed_pages: u32,
    committed: [u64; BITMAP_WORDS],
}

const EMPTY_REGION: Region = Region {
    active: false,
    id: 0,
    pid: 0,
    base: 0,
    pages: 0,
    committed_pages: 0,
    committed: [0; BITMAP_WORDS],
};

struct Registry {
    next_id: u64,
    committed_pages: u32,
    regions: [Region; MAX_REGIONS],
}

static REGISTRY: SyncCell<Registry> = SyncCell::new(Registry {
    next_id: 1,
    committed_pages: 0,
    regions: [EMPTY_REGION; MAX_REGIONS],
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub cap_slot: usize,
    pub id: u32,
    pub base: u64,
    pub pages: u32,
}

fn valid_protection(protection: u64) -> bool {
    protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) == 0
        && protection & PROT_READ != 0
        && !(protection & PROT_WRITE != 0 && protection & PROT_EXEC != 0)
}

fn is_committed(region: &Region, page: u32) -> bool {
    region.committed[page as usize / 64] & (1u64 << (page % 64)) != 0
}

fn set_committed(region: &mut Region, page: u32) {
    let word = &mut region.committed[page as usize / 64];
    let bit = 1u64 << (page % 64);
    assert!(*word & bit == 0, "VM committed-page double insertion");
    *word |= bit;
    region.committed_pages += 1;
}

fn process_committed(registry: &Registry, pid: u64) -> u32 {
    registry
        .regions
        .iter()
        .filter(|region| region.active && region.pid == pid)
        .map(|region| region.committed_pages)
        .sum()
}

fn find_region(registry: &Registry, pid: u64, id: u32) -> Option<usize> {
    registry
        .regions
        .iter()
        .position(|region| region.active && region.pid == pid && region.id == id)
}

fn region_bounds(base: u64, pages: u32) -> Option<(u64, u64)> {
    let lo = base.checked_sub(paging::PAGE)?;
    let body = u64::from(pages).checked_mul(paging::PAGE)?;
    let hi = base.checked_add(body)?.checked_add(paging::PAGE)?;
    Some((lo, hi))
}

/// Create a guarded, lazy reservation in the caller's own process and mint
/// its non-copyable capability. No physical frames are allocated here.
pub fn reserve(pid: u64, pages: u32) -> Result<Reservation, VmError> {
    if pages == 0 || pages > MAX_REGION_PAGES {
        return Err(VmError::BadArgument);
    }
    without_interrupts(|| {
        let (registry_slot, id, process_region_count) = unsafe {
            let registry = &*REGISTRY.get();
            let live_for_process = registry
                .regions
                .iter()
                .filter(|region| region.active && region.pid == pid)
                .count();
            if live_for_process >= MAX_REGIONS_PER_PROCESS {
                return Err(VmError::Quota);
            }
            let Some(slot) = registry.regions.iter().position(|region| !region.active) else {
                return Err(VmError::Quota);
            };
            if registry.next_id > u32::MAX as u64 {
                return Err(VmError::Quota);
            }
            let process_region_count = crate::proc::user_regions(pid)
                .ok_or(VmError::BadArgument)?
                .iter()
                .filter(|&&(lo, hi)| lo != 0 || hi != 0)
                .count();
            (slot, registry.next_id as u32, process_region_count)
        };
        if process_region_count >= crate::sched::USER_REGIONS_MAX {
            return Err(VmError::Quota);
        }

        let Some(root) = crate::proc::pml4_of(pid) else {
            return Err(VmError::BadArgument);
        };
        let regions = crate::proc::user_regions(pid).ok_or(VmError::BadArgument)?;
        let body_bytes = u64::from(pages) * paging::PAGE;
        let mut chosen = None;
        for index in 0..MAX_REGIONS {
            let base = VM_BASE + (index as u64 + 1) * VM_SLOT_STRIDE;
            let Some((lo, hi)) = region_bounds(base, pages) else {
                continue;
            };
            if hi > VM_ARENA_END {
                continue;
            }
            let overlaps_process = regions
                .iter()
                .any(|&(start, end)| start != 0 && lo < end && start < hi);
            if overlaps_process {
                continue;
            }
            let overlaps_vm = unsafe {
                (*REGISTRY.get()).regions.iter().any(|region| {
                    if !region.active || region.pid != pid {
                        return false;
                    }
                    let Some((start, end)) = region_bounds(region.base, region.pages) else {
                        return true;
                    };
                    lo < end && start < hi
                })
            };
            if !overlaps_vm {
                chosen = Some((base, lo, hi));
                break;
            }
        }
        let Some((base, lo, hi)) = chosen else {
            return Err(VmError::Busy);
        };
        // The body is below the canonical user/kernel split and both guard
        // pages are kept inside this exclusive registered span.
        if base < VM_BASE || base.checked_add(body_bytes).is_none_or(|end| end >= hi) {
            return Err(VmError::BadAddress);
        }
        if let Err(_) = crate::proc::append_user_region(pid, lo, hi) {
            return Err(VmError::Quota);
        }

        let grant = crate::cap::grant(
            pid,
            Cap {
                obj: CapObj::VmRegion { id },
                rights: RIGHTS_READ | RIGHTS_WRITE | RIGHTS_DESTROY,
            },
        );
        let cap_slot = match grant {
            Ok(slot) => slot,
            Err(_) => {
                crate::proc::remove_user_region(pid, lo, hi)
                    .unwrap_or_else(|_| crate::halt::halt_machine("VM reserve rollback range"));
                return Err(VmError::Busy);
            }
        };

        let _ = root; // Root liveness was checked before publishing the span.
        unsafe {
            let registry = &mut *REGISTRY.get();
            registry.regions[registry_slot] = Region {
                active: true,
                id,
                pid,
                base,
                pages,
                committed_pages: 0,
                committed: [0; BITMAP_WORDS],
            };
            registry.next_id += 1;
        }
        Ok(Reservation {
            cap_slot,
            id,
            base,
            pages,
        })
    })
}

/// Commit zeroed private pages into an exact, owned reservation. All resource
/// and range checks precede page-table mutation.
pub fn commit(pid: u64, id: u32, offset: u32, pages: u32, protection: u64) -> Result<(), VmError> {
    if pages == 0 || pages > MAX_OPERATION_PAGES || !valid_protection(protection) {
        return Err(VmError::BadArgument);
    }
    without_interrupts(|| {
        let region = unsafe {
            let registry = &*REGISTRY.get();
            let Some(index) = find_region(registry, pid, id) else {
                return Err(VmError::BadArgument);
            };
            registry.regions[index]
        };
        let end_page = offset.checked_add(pages).ok_or(VmError::BadArgument)?;
        if end_page > region.pages {
            return Err(VmError::BadArgument);
        }
        if (offset..end_page).any(|page| is_committed(&region, page)) {
            return Err(VmError::Busy);
        }
        let registry = unsafe { &*REGISTRY.get() };
        if process_committed(registry, pid).saturating_add(pages) > MAX_COMMITTED_PER_PROCESS
            || registry.committed_pages.saturating_add(pages) > MAX_COMMITTED_PAGES
        {
            return Err(VmError::Quota);
        }
        let Some(root) = crate::proc::pml4_of(pid) else {
            return Err(VmError::BadArgument);
        };
        let va = region.base + u64::from(offset) * paging::PAGE;
        let table_frames =
            paging::page_table_frames_worst(va, u64::from(pages)).ok_or(VmError::BadAddress)?;
        if frames::free_frames() < u64::from(pages) + table_frames {
            return Err(VmError::Busy);
        }
        for page in 0..pages {
            // Reservation ownership prevents all other native map paths from
            // entering this range; the probe also catches a kernel bug before
            // any backing frame is claimed.
            if unsafe { paging::user_va_mapped(root, va + u64::from(page) * paging::PAGE) } {
                return Err(VmError::Busy);
            }
        }

        let mut backing = [0u64; MAX_OPERATION_PAGES as usize];
        for index in 0..pages as usize {
            let Some(frame) = frames::alloc() else {
                for allocated in backing[..index].iter().copied() {
                    frames::free(allocated)
                        .unwrap_or_else(|_| crate::halt::halt_machine("VM commit rollback free"));
                }
                return Err(VmError::Busy);
            };
            backing[index] = frame;
        }

        for index in 0..pages as usize {
            let page_va = va + index as u64 * paging::PAGE;
            // SAFETY: free-frame preflight reserved enough backing and page
            // table frames; IF=0, the exact private reservation is unmapped.
            // Map RW/NX temporarily so zeroing works with CR0.WP enabled.
            unsafe {
                paging::map_user_page_4k(root, page_va, backing[index], true, false)
                    .unwrap_or_else(|_| crate::halt::halt_machine("VM commit map after preflight"));
                crate::arch::x86_64::invlpg(page_va);
                crate::arch::x86_64::stac();
                core::ptr::write_bytes(page_va as *mut u8, 0, paging::PAGE as usize);
                crate::arch::x86_64::clac();
            }
        }
        let writable = protection & PROT_WRITE != 0;
        let exec = protection & PROT_EXEC != 0;
        if !writable || exec {
            for index in 0..pages as usize {
                let page_va = va + index as u64 * paging::PAGE;
                // SAFETY: newly installed private page; final permissions
                // preserve W^X and the range is not yet published to userspace.
                unsafe {
                    paging::protect_user_page_4k(root, page_va, writable, exec)
                        .unwrap_or_else(|_| crate::halt::halt_machine("VM commit protection"));
                }
            }
        }

        unsafe {
            let registry = &mut *REGISTRY.get();
            let index = find_region(registry, pid, id)
                .unwrap_or_else(|| crate::halt::halt_machine("VM commit reservation vanished"));
            for page in offset..end_page {
                set_committed(&mut registry.regions[index], page);
            }
            registry.committed_pages += pages;
        }
        Ok(())
    })
}

/// Change permissions for a fully committed range. The entire range is
/// checked before any leaf changes, and W^X is rejected by construction.
pub fn protect(pid: u64, id: u32, offset: u32, pages: u32, protection: u64) -> Result<(), VmError> {
    if pages == 0 || pages > MAX_OPERATION_PAGES || !valid_protection(protection) {
        return Err(VmError::BadArgument);
    }
    without_interrupts(|| {
        let region = unsafe {
            let registry = &*REGISTRY.get();
            let Some(index) = find_region(registry, pid, id) else {
                return Err(VmError::BadArgument);
            };
            registry.regions[index]
        };
        let end_page = offset.checked_add(pages).ok_or(VmError::BadArgument)?;
        if end_page > region.pages {
            return Err(VmError::BadArgument);
        }
        if (offset..end_page).any(|page| !is_committed(&region, page)) {
            return Err(VmError::BadArgument);
        }
        let Some(root) = crate::proc::pml4_of(pid) else {
            return Err(VmError::BadArgument);
        };
        let va = region.base + u64::from(offset) * paging::PAGE;
        for page in 0..pages {
            let page_va = va + u64::from(page) * paging::PAGE;
            let Some(flags) = (unsafe { paging::user_pte_flags(root, page_va) }) else {
                return Err(VmError::BadAddress);
            };
            let phys = flags & 0x000F_FFFF_FFFF_F000;
            if flags & paging::PTE_USER == 0
                || flags & (paging::PTE_PCD | paging::PTE_PWT) != 0
                || crate::shared::is_backing(phys)
            {
                return Err(VmError::BadAddress);
            }
        }
        let writable = protection & PROT_WRITE != 0;
        let exec = protection & PROT_EXEC != 0;
        for page in 0..pages {
            let page_va = va + u64::from(page) * paging::PAGE;
            // SAFETY: full-range preflight above; no other CPU or thread can
            // modify this process's PTEs in the IF=0 syscall window.
            unsafe {
                paging::protect_user_page_4k(root, page_va, writable, exec)
                    .unwrap_or_else(|_| crate::halt::halt_machine("VM protect after preflight"));
            }
        }
        Ok(())
    })
}

/// Destroy one exact reservation cap's backing. The caller has checked its
/// DESTROY right; failure before mutation leaves the region and cap intact.
pub fn release(pid: u64, id: u32) -> Result<(), VmError> {
    without_interrupts(|| {
        let region = unsafe {
            let registry = &*REGISTRY.get();
            let Some(index) = find_region(registry, pid, id) else {
                return Err(VmError::BadArgument);
            };
            registry.regions[index]
        };
        let (lo, hi) = region_bounds(region.base, region.pages).ok_or(VmError::BadAddress)?;
        let ranges = crate::proc::user_regions(pid).ok_or(VmError::BadArgument)?;
        if !ranges.contains(&(lo, hi)) {
            return Err(VmError::BadAddress);
        }
        let Some(root) = crate::proc::pml4_of(pid) else {
            return Err(VmError::BadArgument);
        };
        for page in 0..region.pages {
            if !is_committed(&region, page) {
                continue;
            }
            let va = region.base + u64::from(page) * paging::PAGE;
            let Some(flags) = (unsafe { paging::user_pte_flags(root, va) }) else {
                return Err(VmError::BadAddress);
            };
            let phys = flags & 0x000F_FFFF_FFFF_F000;
            if flags & paging::PTE_USER == 0
                || flags & (paging::PTE_PCD | paging::PTE_PWT) != 0
                || crate::shared::is_backing(phys)
            {
                return Err(VmError::BadAddress);
            }
        }

        for page in 0..region.pages {
            if !is_committed(&region, page) {
                continue;
            }
            let va = region.base + u64::from(page) * paging::PAGE;
            // SAFETY: every committed private leaf was preflighted above.
            let phys = unsafe { paging::unmap_owned_user_page_4k(root, va) }
                .unwrap_or_else(|| crate::halt::halt_machine("VM release leaf vanished"));
            frames::free(phys)
                .unwrap_or_else(|_| crate::halt::halt_machine("VM release frame free refused"));
        }
        crate::proc::remove_user_region(pid, lo, hi)
            .unwrap_or_else(|_| crate::halt::halt_machine("VM release range vanished"));
        unsafe {
            let registry = &mut *REGISTRY.get();
            let index = find_region(registry, pid, id)
                .unwrap_or_else(|| crate::halt::halt_machine("VM release record vanished"));
            registry.committed_pages -= registry.regions[index].committed_pages;
            registry.regions[index] = EMPTY_REGION;
        }
        Ok(())
    })
}

/// Forget metadata after normal page-table teardown already reclaimed all
/// user leaves. This deliberately performs no physical frees.
pub fn forget_process(pid: u64) {
    without_interrupts(|| unsafe {
        let registry = &mut *REGISTRY.get();
        for region in &mut registry.regions {
            if region.active && region.pid == pid {
                registry.committed_pages -= region.committed_pages;
                *region = EMPTY_REGION;
            }
        }
    });
}

pub fn live(pid: u64, id: u32) -> bool {
    without_interrupts(|| unsafe { find_region(&*REGISTRY.get(), pid, id).is_some() })
}

/// `[base, capacity pages, region committed pages, global committed pages,
/// guard pages, global live regions]`; callers must hold the exact region cap.
pub fn query(pid: u64, id: u32) -> Option<[u64; 6]> {
    without_interrupts(|| unsafe {
        let registry = &*REGISTRY.get();
        let index = find_region(registry, pid, id)?;
        let region = registry.regions[index];
        Some([
            region.base,
            u64::from(region.pages),
            u64::from(region.committed_pages),
            u64::from(registry.committed_pages),
            GUARD_PAGES,
            registry.regions.iter().filter(|entry| entry.active).count() as u64,
        ])
    })
}

pub fn usage_snapshot() -> (usize, u32) {
    without_interrupts(|| unsafe {
        let registry = &*REGISTRY.get();
        (
            registry
                .regions
                .iter()
                .filter(|region| region.active)
                .count(),
            registry.committed_pages,
        )
    })
}
