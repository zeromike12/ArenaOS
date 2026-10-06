//! ADR-0056: bounded generic shared-RAM registry, independent of graphics.
//!
//! Full, never-reused IDs; one physical owner per contiguous run. Ordinary
//! copied/IPC-staged capabilities are references, not independent owners.
//! Mapping pins survive cap destruction; the process teardown walks the
//! page tables *before* dropping mapping pins. No numeric ID grants access.

use crate::arch::x86_64::paging;
use crate::cap::{Cap, CapObj};
use crate::frames;
use crate::sync::{SyncCell, without_interrupts};

// ADR-0075 (Phase 11.3): twelve desktop sessions, each with one shared
// surface reservation and one private snapshot region (mapped, never
// capability-held), plus the scanout and the historical fixtures.
pub const MAX_REGIONS: usize = 80;
/// One work-area surface reservation on the largest supported screen
/// (1024x768) plus its transient-surface area fits in one region.
pub const MAX_PAGES: u32 = 1024;
pub const TOTAL_PAGES: u32 = 36864;
pub const MAX_MAPS: usize = 128;

/// Read-only bounded resource snapshot for boot diagnostics. This never
/// grants access to region IDs, physical addresses or mapped pages.
pub fn usage_snapshot() -> (usize, u32, usize) {
    without_interrupts(|| {
        // SAFETY: the registry is read while IF=0, with no held mutable
        // borrow or other CPU writing in this single-CPU implementation.
        let r = unsafe { &*REG.get() };
        (
            r.regions.iter().filter(|region| region.pages != 0).count(),
            r.used_pages,
            r.maps.iter().filter(|map| map.id != 0).count(),
        )
    })
}

/// Root-only, read-only stable-boundary witness for an exact full region ID.
/// Never used to authorize a user request; only the original root-allocated
/// region's own cap and mapping hooks can change these counters. The caller
/// must independently check the conservation oracle before retirement.
pub fn reference_snapshot(id: u32) -> Option<(u32, u32)> {
    without_interrupts(|| unsafe {
        (*REG.get())
            .regions
            .iter()
            .find(|r| r.pages != 0 && r.id == id)
            .map(|r| (r.refs, r.pins))
    })
}

#[derive(Clone, Copy)]
struct Region {
    id: u32,
    phys: u64,
    pages: u32,
    refs: u32,
    pins: u32,
}
const EMPTY_REGION: Region = Region {
    id: 0,
    phys: 0,
    pages: 0,
    refs: 0,
    pins: 0,
};
#[derive(Clone, Copy)]
struct Mapping {
    id: u32,
    pid: u64,
    va: u64,
}
const EMPTY_MAP: Mapping = Mapping {
    id: 0,
    pid: 0,
    va: 0,
};
struct Registry {
    next_id: u64,
    used_pages: u32,
    regions: [Region; MAX_REGIONS],
    maps: [Mapping; MAX_MAPS],
}
static REG: SyncCell<Registry> = SyncCell::new(Registry {
    next_id: 1,
    used_pages: 0,
    regions: [EMPTY_REGION; MAX_REGIONS],
    maps: [EMPTY_MAP; MAX_MAPS],
});

fn release_empty(r: &mut Registry, idx: usize) {
    let entry = r.regions[idx];
    if entry.pages == 0 || entry.refs != 0 || entry.pins != 0 {
        return;
    }
    frames::free_contiguous(entry.phys, entry.pages as usize)
        .unwrap_or_else(|_| crate::halt::halt_machine("SharedRegion double/free invalid run"));
    r.used_pages -= entry.pages;
    r.regions[idx] = EMPTY_REGION;
}

/// Preflight has already checked free cap slot and destination output.
/// Allocation refusal makes *no* registry/ID/cap mutation. Zeroed pages
/// are never visible to any process until `cap::grant` succeeds.
pub fn create(pid: u64, pages: u32) -> Option<(usize, u32)> {
    if pages == 0 || pages > MAX_PAGES {
        return None;
    }
    without_interrupts(|| {
        // End the registry borrow BEFORE cap::grant calls the add_cap hook.
        let (idx, id) = unsafe {
            let r = &mut *REG.get();
            if r.used_pages.checked_add(pages)? > TOTAL_PAGES || r.next_id > u32::MAX as u64 {
                return None;
            }
            let idx = r.regions.iter().position(|e| e.pages == 0)?;
            let phys = frames::alloc_contiguous(pages as usize)?;
            let size = u64::from(pages) * frames::FRAME_BYTES;
            // The current kernel exposes a direct map of only low 2 GiB.
            if phys
                .checked_add(size)
                .is_none_or(|end| end > paging::DIRECT_MAP_BYTES)
            {
                frames::free_contiguous(phys, pages as usize).expect("allocation rollback");
                return None;
            }
            core::ptr::write_bytes((phys + paging::KERNEL_OFFSET) as *mut u8, 0, size as usize);
            let id = r.next_id as u32;
            r.regions[idx] = Region {
                id,
                phys,
                pages,
                refs: 0,
                pins: 0,
            };
            (idx, id)
        };
        // IF=0: cap::grant cannot lose the preflighted slot, but check it
        // anyway so an internal integration bug fails loudly, not as a leak.
        let slot = crate::cap::grant(
            pid,
            Cap {
                obj: CapObj::SharedRegion { id },
                rights: crate::cap::RIGHTS_READ
                    | crate::cap::RIGHTS_WRITE
                    | crate::cap::RIGHTS_COPY
                    | crate::cap::RIGHTS_DESTROY,
            },
        )
        .unwrap_or_else(|_| crate::halt::halt_machine("SharedRegion grant after preflight"));
        unsafe {
            let r = &mut *REG.get();
            assert_eq!(r.regions[idx].refs, 1);
            r.next_id += 1;
            r.used_pages += pages;
        }
        Some((slot, id))
    })
}

/// Mutators of the cap table and IPC queues use identical hooks. A staged
/// cap counts exactly once while in escrow and once in the landed table;
/// adding before dropping prevents premature physical retirement.
pub fn add_cap(cap: Cap) {
    if let CapObj::SharedRegion { id } = cap.obj {
        without_interrupts(|| unsafe {
            let r = &mut *REG.get();
            let entry = r
                .regions
                .iter_mut()
                .find(|e| e.pages != 0 && e.id == id)
                .expect("SharedRegion cap names retired generation");
            entry.refs = entry
                .refs
                .checked_add(1)
                .expect("SharedRegion ref overflow");
        });
    }
}
pub fn drop_cap(cap: Cap) {
    if let CapObj::SharedRegion { id } = cap.obj {
        without_interrupts(|| unsafe {
            let r = &mut *REG.get();
            let idx = r
                .regions
                .iter()
                .position(|e| e.pages != 0 && e.id == id)
                .expect("SharedRegion cap dropped after retirement");
            assert!(r.regions[idx].refs > 0);
            r.regions[idx].refs -= 1;
            release_empty(r, idx);
        });
    }
}
pub fn backing(id: u32) -> Option<(u64, u32)> {
    without_interrupts(|| unsafe {
        (*REG.get())
            .regions
            .iter()
            .find(|e| e.pages != 0 && e.id == id)
            .map(|e| (e.phys, e.pages))
    })
}
/// Exact own-mapping lookup; no numeric ID lookup or raw physical address
/// is exposed to userland. SYS_SHARED_UNMAP preflights this against both
/// the thread's registered user span and every owned PTE before mutation.
pub fn own_mapping(pid: u64, va: u64) -> Option<(u32, u64, u32)> {
    without_interrupts(|| unsafe {
        let r = &*REG.get();
        let m = r
            .maps
            .iter()
            .find(|m| m.id != 0 && m.pid == pid && m.va == va)?;
        let entry = r.regions.iter().find(|e| e.pages != 0 && e.id == m.id)?;
        Some((m.id, entry.phys, entry.pages))
    })
}

/// Called only after the exact PTE run and user region were removed at
/// IF=0. Last-pin retirement cannot occur while any cap/escrow remains.
pub fn unpin_own_mapping(pid: u64, va: u64, id: u32) {
    without_interrupts(|| unsafe {
        let r = &mut *REG.get();
        let mi = r
            .maps
            .iter()
            .position(|m| m.id == id && m.pid == pid && m.va == va)
            .expect("SharedRegion mapping disappeared after PTE preflight");
        let ri = r
            .regions
            .iter()
            .position(|e| e.pages != 0 && e.id == id)
            .expect("SharedRegion region disappeared while mapped");
        assert!(r.regions[ri].pins > 0);
        r.maps[mi] = EMPTY_MAP;
        r.regions[ri].pins -= 1;
        release_empty(r, ri);
    });
}

pub fn has_map_slot() -> bool {
    without_interrupts(|| unsafe { (*REG.get()).maps.iter().any(|m| m.id == 0) })
}
/// Called only *after* mapping succeeded and before syscall return (IF=0).
pub fn pin_map(id: u32, pid: u64, va: u64) {
    without_interrupts(|| unsafe {
        let r = &mut *REG.get();
        let entry = r
            .regions
            .iter_mut()
            .find(|e| e.pages != 0 && e.id == id)
            .expect("SharedRegion map target disappeared");
        entry.pins = entry
            .pins
            .checked_add(1)
            .expect("SharedRegion pin overflow");
        let slot = r
            .maps
            .iter_mut()
            .find(|m| m.id == 0)
            .expect("SharedRegion map table full");
        *slot = Mapping { id, pid, va };
    });
}
/// Page-table leaves are still physically pinned when all caps disappear;
/// release the pins only AFTER the process's PTE teardown skipped them.
pub fn release_maps(pid: u64) {
    without_interrupts(|| unsafe {
        let r = &mut *REG.get();
        for i in 0..MAX_MAPS {
            let m = r.maps[i];
            if m.id != 0 && m.pid == pid {
                let idx = r
                    .regions
                    .iter()
                    .position(|e| e.pages != 0 && e.id == m.id)
                    .expect("SharedRegion mapping names retired generation");
                assert!(r.regions[idx].pins > 0);
                r.regions[idx].pins -= 1;
                r.maps[i] = EMPTY_MAP;
                release_empty(r, idx);
            }
        }
    });
}

/// Paging teardown calls this for each user leaf, not just through the
/// original mapping VA. No physical frame belonging to a live shared run
/// may be returned to the allocator by the generic process walk.
pub fn is_backing(phys: u64) -> bool {
    without_interrupts(|| unsafe {
        (*REG.get())
            .regions
            .iter()
            .any(|e| e.pages != 0 && phys >= e.phys && phys < e.phys + u64::from(e.pages) * 4096)
    })
}

/// Independent stable-boundary oracle: count real process and IPC cap
/// references (not the ref hooks) and verify each mapping against a
/// present PTE in the actual address space and the full ID. The map list
/// cannot substitute for the page-table walk.
pub fn assert_conservation() {
    without_interrupts(|| unsafe {
        let r = &*REG.get();
        let mut total = 0u32;
        for e in &r.regions {
            if e.pages == 0 {
                continue;
            }
            total += e.pages;
            let mut refs = 0u32;
            let mut count = |cap: Cap| {
                if cap.obj == (CapObj::SharedRegion { id: e.id }) {
                    refs = refs.checked_add(1).expect("SharedRegion oracle overflow");
                }
            };
            crate::proc::for_each_cap(&mut count);
            crate::ipc::for_each_staged_cap(&mut count);
            let mut pins = 0u32;
            for m in r.maps.iter().filter(|m| m.id == e.id) {
                pins += 1;
                let root = crate::proc::pml4_of(m.pid).expect("SharedRegion map pins dead process");
                for i in 0..u64::from(e.pages) {
                    let pte = paging::user_pte_flags(root, m.va + i * 4096)
                        .expect("SharedRegion map PTE missing");
                    if pte & 0x000f_ffff_ffff_f000 != e.phys + i * 4096 {
                        crate::halt::halt_machine("SharedRegion map PTE mismatches backing");
                    }
                }
            }
            if refs != e.refs || pins != e.pins || refs == 0 && pins == 0 {
                crate::halt::halt_machine("SharedRegion ref/map conservation failure");
            }
        }
        assert_eq!(r.used_pages, total, "SharedRegion page conservation");
    });
}
