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
    /// An IPC endpoint (ADR-0018): rendezvous point for synchronous
    /// call/reply. Rights: WRITE = call side, READ = serve side
    /// (recv/reply). The object lives in `ipc::ENDPOINTS`; the cap
    /// references it by index — destroying the cap frees nothing.
    Endpoint { eid: u32 },
    /// OWNED physical frame (ADR-0021) — the driver-substrate primitive.
    /// The holder owns exactly one allocator frame: `destroy` returns it;
    /// `map_memory` TRANSFERS ownership into the target's address space
    /// and consumes the cap (the teardown walk then reclaims the frame
    /// when the process dies). Exactly one owner at any time, so a frame
    /// is freed exactly once either way.
    Untyped { phys: u64 },
    /// Kernel-minted device register window (ADR-0021): MMIO physical
    /// base + page count. Descriptive — never owned, frees nothing, never
    /// executable; ring 3 can only receive one from a kernel scan or the
    /// kernel's own test suites.
    Mmio { phys: u64, pages: u32 },
    /// A badged, merged notification flag word (ADR-0018, ARCHITECTURE
    /// §7.2). Rights: WRITE = notify, READ = wait.
    Notification { nid: u32 },
    /// A registered executable image (ADR-0019): the thing `SYS_SPAWN`
    /// builds processes from. Rights: READ = may spawn from it. v1's
    /// registry is kernel-side and fixed; a filesystem-backed source
    /// arrives later without changing this shape.
    Image { img_id: u32 },
    /// The machine-power singleton (ADR-0020): WRITE = may halt the
    /// machine through `SYS_SHUTDOWN`. One kernel object, no identity —
    /// holding the cap with the right IS the authority. The kernel's
    /// own panic/suite halt paths are ring-0 internals, not invokes of
    /// this object.
    Power,
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

/// Kernel-side issuance (ADR-0021): install a freshly minted cap into
/// `pid`'s `slot`. Refuses to clobber an occupied slot — issuance must
/// never silently drop (and leak) a live cap; the caller picks free
/// slots. This is the only public write path that does not move or
/// attenuate an existing cap (`SYS_ALLOC_FRAME` mints Untyped caps
/// through it).
pub fn issue(pid: u64, slot: usize, cap: Cap) -> Result<(), &'static str> {
    without_interrupts(|| {
        if read(pid, slot).is_ok() {
            return Err("cap issue: slot occupied");
        }
        install(pid, slot, cap)
    })
}

/// Consume an OWNED cap whose object was just transferred (ADR-0021:
/// `SYS_MAP_MEMORY` moves an Untyped cap's frame into the address
/// space). Kernel-side: the caller has already validated kind, rights,
/// and the transfer itself; this only empties the slot.
pub fn consume(pid: u64, slot: usize) -> Result<(), &'static str> {
    install(pid, slot, Cap::EMPTY)
}

/// Remove the cap from its slot — the *reference*, not the object
/// (ADR-0015: object lifetime belongs to the owning subsystem; caps may
/// dangle and invokes re-validate). Requires the `DESTROY` right.
///
/// Kind-aware since ADR-0021: destroying an [`CapObj::Untyped`] cap
/// returns its frame to the allocator — exactly once, because mapping
/// such a cap consumes it (ownership moves to the address space, whose
/// teardown walk reclaims the frame instead). All other kinds are
/// descriptive and free nothing.
pub fn destroy(pid: u64, slot: usize) -> Result<(), &'static str> {
    without_interrupts(|| {
        let cap = read(pid, slot)?;
        if cap.rights & RIGHTS_DESTROY == 0 {
            return Err("cap destroy: cap lacks the DESTROY right");
        }
        // Owned frame: return it. If the allocator refuses (a state bug —
        // the frame is not ours to free), the slot is NOT cleared: the
        // loud refusal beats a silent leak.
        if let CapObj::Untyped { phys } = cap.obj {
            crate::frames::free(phys).map_err(|_| "cap destroy: untyped frame free refused")?;
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

/// Gated invoke: map a memory-kind cap's frames into the user half of
/// the process a `Process` cap names — the untyped-memory →
/// address-space binding of ARCHITECTURE §4, in miniature. Both caps
/// must live in the *same* space (`pid`); the process cap needs `WRITE`
/// and a live target. `va` is page-aligned, lower-half, and must have
/// room for all `pages`; W^X flags are the caller's (`writable && exec`
/// is rejected by the mapper, ADR-0008).
///
/// Memory-cap gates, relaxed by ADR-0021 (the old WRITE-for-everything
/// rule made read-only descriptor windows impossible):
///
/// * the requested access mode decides the right: `writable` needs
///   `WRITE`, a read-only window needs `READ`;
/// * [`CapObj::Untyped`] (one owned frame) additionally needs `DESTROY`
///   and is CONSUMED on success — ownership transfers to the target's
///   address space, whose teardown walk reclaims the frame;
/// * [`CapObj::Untyped`]/[`CapObj::Mmio`] windows are never executable.
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
        // Normalize the memory kinds (ADR-0021): Memory/Mmio describe
        // (phys, pages); Untyped is one OWNED frame — mapping it
        // transfers ownership, so success consumes the cap.
        let (phys, pages, owned) = match mem.obj {
            CapObj::Memory { phys, pages } => (phys, pages, false),
            CapObj::Mmio { phys, pages } => (phys, pages, false),
            CapObj::Untyped { phys } => (phys, 1, true),
            _ => return Err("map_memory: memory slot does not name a memory cap"),
        };
        // The access mode decides the right (ADR-0021): a writable
        // window needs WRITE, a read-only window needs READ.
        if writable {
            if mem.rights & RIGHTS_WRITE == 0 {
                return Err("map_memory: memory cap lacks the WRITE right");
            }
        } else if mem.rights & RIGHTS_READ == 0 {
            return Err("map_memory: memory cap lacks the READ right");
        }
        if owned {
            if mem.rights & RIGHTS_DESTROY == 0 {
                return Err("map_memory: untyped map consumes the cap — DESTROY required");
            }
            if exec {
                return Err("map_memory: untyped/mmio windows are never executable");
            }
        }
        if exec && matches!(mem.obj, CapObj::Mmio { .. }) {
            return Err("map_memory: untyped/mmio windows are never executable");
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
        // Ownership transfer (ADR-0021): the consumed Untyped cap's slot
        // goes empty — the frame now belongs to the target's address
        // space and is reclaimed by its teardown walk.
        if owned {
            install(pid, mem_slot, Cap::EMPTY)?;
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
