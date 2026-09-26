//! Capability spaces (ROADMAP 3.4, ADR-0015): the reference model for
//! kernel objects. A capability is a *name* — `(object reference, rights)`
//! — living in a fixed per-process slot table embedded in [`crate::proc`]
//! (`Process.caps`). Nothing references a kernel object except through a
//! validated slot in some space, and every operation re-checks bounds,
//! occupancy, rights, and the referenced object's liveness at invoke time.
//!
//! Security invariants (all tested by the m3 suite):
//! * delegation attenuates only — `copy`/`move` demand an explicit rights
//!   subset of the source's and `COPY` on the source; amplification is a
//!   loud `Err`, never a silent clamp;
//! * destroy removes the *reference*, not the object — object lifetime
//!   stays with its owning subsystem, so caps can dangle and every invoke
//!   re-validates liveness (`proc::pml4_of` for process caps);
//! * rights gate real actions: `process_root` (READ — the target's PML4
//!   PHYS is information) and `map_memory` (WRITE on a memory cap *and*
//!   WRITE on a live process cap — mapping untyped frames into an address
//!   space is the mutation this model exists to control).
//!
//! Kernel-internal only at this milestone: the sole writer path is kernel
//! code under IF=0 (`grant` is the boot/root-task trust primitive). The
//! M4 syscall surface will pass slot indices against the calling thread's
//! process space and add unmarshalling — the validation core below is
//! already that surface's semantics.

use crate::arch::x86_64::paging;
use crate::frames;
use crate::proc;
use crate::sync::without_interrupts;

/// Slots per capability space. Fixed capacity, no growth — the M3
/// discipline (threads, processes) applied again; revisited when
/// userspace cspaces need more (ADR-0015, Future implications).
pub const CAP_SLOTS: usize = 16;

/// Inspect what the cap references (and, for process caps, obtain the
/// target's PML4 root through [`process_root`]).
pub const RIGHTS_READ: u32 = 1 << 0;
/// Mutate the referenced object through an invoke ([`map_memory`]).
pub const RIGHTS_WRITE: u32 = 1 << 1;
/// Delegate: required *on the source* of [`copy`]/[`move_cap`].
pub const RIGHTS_COPY: u32 = 1 << 2;
/// Remove the cap from its slot ([`destroy`]).
pub const RIGHTS_DESTROY: u32 = 1 << 3;
/// Every right defined at this milestone.
pub const RIGHTS_ALL: u32 = RIGHTS_READ | RIGHTS_WRITE | RIGHTS_COPY | RIGHTS_DESTROY;

/// The object a capability names. Both kinds are real kernel objects
/// today: address spaces (M3.3, `proc.rs`) and untyped-style physical
/// memory ranges (the seed of the ARCHITECTURE §4 untyped ABI). New
/// kinds (endpoints, threads, untyped-kernel-allocated memory) extend
/// this enum; the slot/rights machinery is kind-agnostic.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CapObj {
    /// Empty slot marker (the space is a plain array; `None` *is* the
    /// option type — no `Option<Cap>` size games).
    None,
    /// A process address-space object, by pid.
    Process { pid: u64 },
    /// Untyped memory: `pages` 4 KiB frames starting at `phys`. The
    /// granter vouches the frames are owned (ADR-0015, Downsides);
    /// the cap *describes* them — it never owns them, so destroying the
    /// cap frees nothing and teardown stays exact.
    Memory { phys: u64, pages: u32 },
}

/// One capability: an object reference plus its rights mask.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Cap {
    pub obj: CapObj,
    pub rights: u32,
}

impl Cap {
    /// The empty capability (slot-free marker; rights are meaningless).
    pub const EMPTY: Cap = Cap {
        obj: CapObj::None,
        rights: 0,
    };
}

/// A process's capability space: the fixed slot table itself. Embedded
/// in [`proc::Process`], so it is born with `proc::create` and dies with
/// `proc::destroy` — the ADR-0014 anchor promise.
#[derive(Clone, Copy)]
pub struct CapSpace {
    slots: [Cap; CAP_SLOTS],
}

impl CapSpace {
    /// All-empty space (`proc::create` starts every process with this).
    pub const fn new() -> Self {
        CapSpace {
            slots: [Cap::EMPTY; CAP_SLOTS],
        }
    }

    fn get(&self, slot: usize) -> Option<Cap> {
        self.slots.get(slot).copied()
    }

    /// Occupied-slot count (computed, never tracked — no sync bug).
    fn used(&self) -> u32 {
        self.slots
            .iter()
            .filter(|c| !matches!(c.obj, CapObj::None))
            .count() as u32
    }
}

/// Read the cap at `(pid, slot)`. Kernel-internal introspection
/// primitive: bounds- and occupancy-checked, no rights gate (the whole
/// module is kernel-only until M4; user-facing reads are invokes).
pub fn read(pid: u64, slot: usize) -> Result<Cap, &'static str> {
    without_interrupts(|| {
        proc::with_caps(pid, |cs| -> Result<Cap, &'static str> {
            let cap = cs.get(slot).ok_or("cap slot out of bounds")?;
            if matches!(cap.obj, CapObj::None) {
                return Err("cap slot empty");
            }
            Ok(cap)
        })
        .ok_or("cap: no such process")?
    })
}

/// Occupancy evidence for the suites: `(used, CAP_SLOTS)` of a live
/// process's space (`None` for an unknown pid).
pub fn occupancy(pid: u64) -> Option<(u32, u32)> {
    without_interrupts(|| proc::with_caps(pid, |cs| (cs.used(), CAP_SLOTS as u32)))
}

/// The root-of-trust grant: install `cap` into the first empty slot of
/// `pid`'s space and return the slot index. This is the boot/root-task
/// policy path (ADR-0015): it may name any object with any rights, and
/// it is the *only* creation path — every other operation derives from
/// an existing cap with attenuated rights.
pub fn grant(pid: u64, cap: Cap) -> Result<usize, &'static str> {
    if matches!(cap.obj, CapObj::None) {
        return Err("grant: cannot grant an empty cap");
    }
    without_interrupts(|| {
        proc::with_caps_mut(pid, |cs| -> Result<usize, &'static str> {
            let Some(slot) = cs.slots.iter().position(|c| matches!(c.obj, CapObj::None)) else {
                return Err("capability space full (CAP_SLOTS)");
            };
            cs.slots[slot] = cap;
            Ok(slot)
        })
        .ok_or("grant: no such process")?
    })
}

/// Delegate by copy: install `(source obj, `rights`)` into
/// `(dst_pid, dst_slot)`. Refuses — loudly, never by clamping — when the
/// source lacks `COPY`, when `rights` is not a subset of the source's
/// (amplification), or when the destination slot is occupied. Source and
/// destination may be the same space (different slots).
pub fn copy(
    src_pid: u64,
    src_slot: usize,
    dst_pid: u64,
    dst_slot: usize,
    rights: u32,
) -> Result<(), &'static str> {
    without_interrupts(|| {
        let src = read(src_pid, src_slot)?;
        if src.rights & RIGHTS_COPY == 0 {
            return Err("cap copy: source lacks the COPY right");
        }
        if rights & !src.rights != 0 {
            return Err("cap copy: rights amplification refused");
        }
        let dst = read_or_empty(dst_pid, dst_slot)?;
        if !matches!(dst.obj, CapObj::None) {
            return Err("cap copy: destination slot occupied");
        }
        install(
            dst_pid,
            dst_slot,
            Cap {
                obj: src.obj,
                rights,
            },
        )
    })
}

/// Delegate by move: [`copy`] semantics, then the source slot is
/// cleared. Atomic under IF=0 — no window where the cap exists twice or
/// not at all.
pub fn move_cap(
    src_pid: u64,
    src_slot: usize,
    dst_pid: u64,
    dst_slot: usize,
    rights: u32,
) -> Result<(), &'static str> {
    without_interrupts(|| {
        copy(src_pid, src_slot, dst_pid, dst_slot, rights)?;
        // The copy proved the source occupied and COPY-righted; the
        // clear cannot fail. `move` semantics: rights go with the cap.
        install(src_pid, src_slot, Cap::EMPTY)
    })
}

/// Remove the cap from its slot — the *reference*, not the object
/// (ADR-0015: object lifetime belongs to the owning subsystem; caps may
/// dangle and invokes re-validate). Requires the `DESTROY` right.
pub fn destroy(pid: u64, slot: usize) -> Result<(), &'static str> {
    without_interrupts(|| {
        let cap = read(pid, slot)?;
        if cap.rights & RIGHTS_DESTROY == 0 {
            return Err("cap destroy: cap lacks the DESTROY right");
        }
        install(pid, slot, Cap::EMPTY)
    })
}

/// Gated invoke: the PML4 root PHYS of the process a `Process` cap
/// names. Requires `READ` on the cap and re-validates the target's
/// liveness — a cap to a destroyed process fails cleanly instead of
/// naming freed frames.
pub fn process_root(pid: u64, slot: usize) -> Result<u64, &'static str> {
    without_interrupts(|| {
        let cap = read(pid, slot)?;
        let CapObj::Process { pid: target } = cap.obj else {
            return Err("process_root: cap does not name a process");
        };
        if cap.rights & RIGHTS_READ == 0 {
            return Err("process_root: cap lacks the READ right");
        }
        proc::pml4_of(target).ok_or("process_root: target process is dead (cap dangles)")
    })
}

/// Gated invoke: map a `Memory` cap's frames into the user half of the
/// process a `Process` cap names — the untyped-memory → address-space
/// binding of ARCHITECTURE §4, in miniature. Both caps must live in the
/// *same* space (`pid`); the memory cap needs `WRITE`, the process cap
/// needs `WRITE` and a live target. `va` is page-aligned, lower-half,
/// and must have room for all `pages`; W^X flags are the caller's
/// (`writable && exec` is rejected by the mapper, ADR-0008).
///
/// Partial failure (a later page refused) leaves earlier pages mapped —
/// the mapper has no undo at this milestone; the suites map into fresh
/// VAs and account frames exactly through `proc::destroy`.
pub fn map_memory(
    pid: u64,
    mem_slot: usize,
    proc_slot: usize,
    va: u64,
    writable: bool,
    exec: bool,
) -> Result<(), &'static str> {
    without_interrupts(|| {
        let mem = read(pid, mem_slot)?;
        let CapObj::Memory { phys, pages } = mem.obj else {
            return Err("map_memory: memory slot does not name a memory cap");
        };
        if mem.rights & RIGHTS_WRITE == 0 {
            return Err("map_memory: memory cap lacks the WRITE right");
        }
        let target_cap = read(pid, proc_slot)?;
        let CapObj::Process { pid: target } = target_cap.obj else {
            return Err("map_memory: process slot does not name a process cap");
        };
        if target_cap.rights & RIGHTS_WRITE == 0 {
            return Err("map_memory: process cap lacks the WRITE right");
        };
        let root =
            proc::pml4_of(target).ok_or("map_memory: target process is dead (cap dangles)")?;
        if pages == 0 {
            return Err("map_memory: memory cap covers no pages");
        }
        let span = u64::from(pages) * paging::PAGE;
        if va % paging::PAGE != 0 || phys % paging::PAGE != 0 {
            return Err("map_memory: unaligned va or phys");
        }
        // Lower-half check with overflow guard: the whole span must fit
        // below the kernel half.
        let Some(va_end) = va.checked_add(span) else {
            return Err("map_memory: va span overflows");
        };
        if va_end > 0x0000_8000_0000_0000 {
            return Err("map_memory: va span reaches the kernel half");
        }
        // SAFETY: IF=0 (enclosing without_interrupts); `root` is the
        // live process's owned PML4 (liveness just re-checked); the
        // frames are the granter-vouched memory the cap describes.
        unsafe {
            for i in 0..u64::from(pages) {
                paging::map_user_page_4k(
                    root,
                    va + i * paging::PAGE,
                    phys + i * frames::FRAME_BYTES,
                    writable,
                    exec,
                )
                .map_err(|_| "map_memory: page mapping refused")?;
            }
        }
        Ok(())
    })
}

/// Bounds-checked slot read that tolerates empties (copy/move's
/// destination probe): `Cap::EMPTY` for a free slot, `Err` for an
/// out-of-range slot or dead process.
fn read_or_empty(pid: u64, slot: usize) -> Result<Cap, &'static str> {
    without_interrupts(|| {
        proc::with_caps(pid, |cs| cs.get(slot).ok_or("cap slot out of bounds"))
            .ok_or("cap: no such process")?
    })
}

/// Write a cap value into a specific slot (bounds-checked). The single
/// mutation point besides `grant`; used by copy/move/destroy so every
/// write goes through one validated path.
fn install(pid: u64, slot: usize, cap: Cap) -> Result<(), &'static str> {
    without_interrupts(|| {
        proc::with_caps_mut(pid, |cs| {
            let Some(entry) = cs.slots.get_mut(slot) else {
                return Err("cap slot out of bounds");
            };
            *entry = cap;
            Ok(())
        })
        .ok_or("cap: no such process")?
    })
}
