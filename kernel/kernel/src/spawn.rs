//! Spawn protocol v1 (M4.5, ADR-0019; kernel-internal root spawn added
//! by M4.6, ADR-0020): process creation from image capabilities, with
//! explicit attenuating handle inheritance and exit-badge notification.
//!
//! Two entry points share one creation sequence:
//! - [`spawn_from`] — the syscall path (`arch::x86_64::syscall::sys_spawn`
//!   owns the ABI surface: cap resolution for the image and notification
//!   handles, the STAC-bracketed read of the inheritance spec from the
//!   parent's memory). Sequence: validate → create → load → stack →
//!   inherit → register → handle → start.
//! - [`spawn_init`] — the boot path (ADR-0020): the same sequence
//!   WITHOUT a parent. No inheritance spec (there is nothing above to
//!   inherit from), no Process handle to grant; the initial service's
//!   caps arrive as kernel literals. This is the seed of the root-task
//!   story — boot-time policy grows here, not in the syscall path.
//!
//! Every partial failure rolls back: a refused spawn costs zero frames
//! and leaves zero objects (the M4.1 double-load discipline applied to
//! process creation).
//!
//! The child's first thread reads its start facts (entry, stack top,
//! user regions) from its spawn record; the test side reads the same
//! records as machine-state evidence of which children exist.

use crate::arch::x86_64::paging;
use crate::arch::x86_64::syscall::{self, STATUS_BAD_ARG, STATUS_BUSY, Status};
use crate::cap::{self, Cap, CapObj};
use crate::elf;
use crate::frames;
use crate::log::log_error as error;
use crate::proc;
use crate::sched;
use crate::sync::{SyncCell, without_interrupts};

/// Image-registry capacity. v1 populates two entries (the embedded
/// rust-lld payload image and the shell); the bound exists so `img_id`
/// is always a checked index, never a trust.
pub const MAX_IMAGES: usize = 4;
/// Most handles one spawn may inherit (the spec arrives in registers +
/// a small user buffer; four is plenty for a supervisor demo and every
/// excess is a typed refusal).
pub const MAX_INHERIT: usize = 4;
/// Spawn-record table bound (one record per spawned child until it is
/// explicitly forgotten).
pub const MAX_SPAWN_RECS: usize = 8;

/// The kernel-side image registry (ADR-0019/0020): image 0 is the
/// embedded test payload — the same bytes the M4.1–M4.3 suites parse,
/// load, and run; image 1 is the shell the boot sequence spawns as the
/// initial service. A filesystem-backed source slots in here later
/// without changing the cap shape.
pub fn image_bytes(img_id: u32) -> Option<&'static [u8]> {
    match img_id {
        0 => Some(elf::TEST_IMAGE),
        1 => Some(elf::SHELL_IMAGE),
        _ => None,
    }
}

/// One spawned child's start facts + identity (the record index is the
/// child thread's spawn argument).
#[derive(Clone, Copy)]
struct SpawnRec {
    live: bool,
    child_pid: u64,
    child_tid: u64,
    entry: u64,
    stack_top: u64,
    /// Page-granular (lo, hi) user regions: the image's segments plus
    /// the derived stack page, zeros filling the rest.
    regions: [(u64, u64); sched::USER_REGIONS_MAX],
}

const EMPTY_REC: SpawnRec = SpawnRec {
    live: false,
    child_pid: 0,
    child_tid: 0,
    entry: 0,
    stack_top: 0,
    regions: [(0, 0); sched::USER_REGIONS_MAX],
};

static RECORDS: SyncCell<[SpawnRec; MAX_SPAWN_RECS]> = SyncCell::new([EMPTY_REC; MAX_SPAWN_RECS]);

/// Machine-state evidence for the suites: `(child_pid, child_tid)` per
/// live record, in table order.
pub fn records_snapshot() -> [Option<(u64, u64)>; MAX_SPAWN_RECS] {
    without_interrupts(|| {
        // SAFETY: single reader under IF=0.
        unsafe { (*RECORDS.get()).map(|r| r.live.then_some((r.child_pid, r.child_tid))) }
    })
}

/// Release a spawn record (teardown-side bookkeeping: v1 has no
/// automatic GC because it has no user-driven child destroy yet).
/// Refuses unknown pids — forgetting a child that was never spawned is
/// a caller bug, not a no-op.
pub fn forget(child_pid: u64) -> Result<(), &'static str> {
    without_interrupts(|| {
        // SAFETY: single writer under IF=0.
        unsafe {
            let Some(rec) = (*RECORDS.get())
                .iter_mut()
                .find(|r| r.live && r.child_pid == child_pid)
            else {
                return Err("forget: no spawn record for that pid");
            };
            *rec = EMPTY_REC;
            Ok(())
        }
    })
}

// ---- the shared creation sequence ------------------------------------------

/// A half-built child: reserved record + created process + loaded image
/// + mapped stack, with all the start facts the record and the first
/// thread need. Both entry points build one, finish it their own way,
/// and roll it back on any refusal.
#[derive(Clone, Copy)]
struct Prepared {
    idx: usize,
    pid: u64,
    entry: u64,
    stack_top: u64,
    regions: [(u64, u64); sched::USER_REGIONS_MAX],
}

impl Prepared {
    /// Undo everything `prepare` built: destroy the process (which
    /// reclaims the image AND stack frames — including partial table
    /// walks — and the cap space with any caps already granted into
    /// it) and release the record. After this, the failed spawn has
    /// cost exactly zero frames and left zero objects.
    fn rollback(self) {
        if let Err(e) = proc::destroy(self.pid) {
            error!("spawn", "rollback: destroy({}) failed: {e}", self.pid);
        }
        // SAFETY: single writer under IF=0.
        without_interrupts(|| unsafe { (*RECORDS.get())[self.idx] = EMPTY_REC });
    }
}

/// Steps 1–3 of the creation sequence: validate the image BEFORE
/// allocating anything, reserve the record, create the process, load
/// the image, derive and map the stack page (first page above the
/// image's top segment VA — derived from the image, never hardcoded,
/// ADR-0019), and compute the page-granular user regions.
fn prepare(img_id: u32) -> Result<Prepared, Status> {
    // 1. Validate BEFORE allocating anything: the registry lookup and
    //    the ADR-0016 validator both run on the caller's kernel stack.
    let bytes = image_bytes(img_id).ok_or(STATUS_BAD_ARG)?;
    let parsed = elf::validate(bytes).map_err(|_| STATUS_BAD_ARG)?;
    // Regions: one per segment + the stack page must fit the thread's
    // region table.
    if parsed.nsegs == 0 || parsed.nsegs + 1 > sched::USER_REGIONS_MAX {
        return Err(STATUS_BAD_ARG);
    }
    let mut top = 0u64;
    for seg in &parsed.segs[..parsed.nsegs] {
        top = top.max(seg.vaddr + seg.memsz);
    }
    let stack_va = top.div_ceil(paging::PAGE) * paging::PAGE;
    let stack_top = stack_va + paging::PAGE;
    let mut regions = [(0u64, 0u64); sched::USER_REGIONS_MAX];
    let mut nr = 0;
    for seg in &parsed.segs[..parsed.nsegs] {
        let hi = (seg.vaddr + seg.memsz).div_ceil(paging::PAGE) * paging::PAGE;
        regions[nr] = (seg.vaddr, hi);
        nr += 1;
    }
    regions[nr] = (stack_va, stack_top);

    // 2. Reserve the record (its index is the child thread's argument).
    // SAFETY: single writer under IF=0.
    let idx = without_interrupts(|| unsafe {
        let recs = &mut *RECORDS.get();
        let Some(i) = recs.iter().position(|r| !r.live) else {
            return None;
        };
        recs[i] = SpawnRec {
            live: true,
            ..EMPTY_REC
        };
        Some(i)
    })
    .ok_or(STATUS_BUSY)?;

    // From here, every failure path must roll back (see
    // Prepared::rollback for the zero-cost guarantee).
    // 3. Child process + image + stack page.
    let child = match proc::create("spawned") {
        Ok(pid) => pid,
        Err(_) => {
            // SAFETY: single writer under IF=0.
            without_interrupts(|| unsafe { (*RECORDS.get())[idx] = EMPTY_REC });
            return Err(STATUS_BUSY); // process table full
        }
    };
    let half = Prepared {
        idx,
        pid: child,
        entry: parsed.entry,
        stack_top,
        regions,
    };
    if elf::load(bytes, child).is_err() {
        half.rollback();
        return Err(STATUS_BAD_ARG);
    }
    // SAFETY: IF=0; the child root is live and owned (just created).
    let Some(root) = proc::pml4_of(child) else {
        half.rollback();
        return Err(STATUS_BAD_ARG);
    };
    let Some(stack_phys) = frames::alloc() else {
        half.rollback();
        return Err(STATUS_BUSY);
    };
    // SAFETY: IF=0; `stack_phys` freshly allocated (exclusive owner);
    // the child root is the live owned PML4 of a process with no
    // threads yet; the VA is free (derived above the image top);
    // RW+NX is the stack's W^X pair.
    let mapped = unsafe {
        core::ptr::write_bytes((stack_phys + paging::KERNEL_OFFSET) as *mut u8, 0, 4096);
        paging::map_user_page_4k(root, stack_va, stack_phys, true, false)
    };
    if mapped.is_err() {
        half.rollback(); // destroy reclaims the stack frame too
        return Err(STATUS_BAD_ARG);
    }
    Ok(half)
}

/// The final steps both entry points share: register the exit
/// notification (fires on the child's last thread exit — the hook lives
/// in SYS_THREAD_EXIT), fill the record's start facts, and start the
/// child's first thread. Rolls the half-built child back on any
/// refusal; returns the child's pid.
fn finish(p: &Prepared, notif: Option<(u32, u64)>) -> Result<u64, Status> {
    if let Some((nid, badge)) = notif {
        if proc::set_exit_notif(p.pid, nid, badge).is_err() {
            p.rollback();
            return Err(STATUS_BAD_ARG);
        }
    }
    // SAFETY: single writer under IF=0.
    without_interrupts(|| unsafe {
        let rec = &mut (*RECORDS.get())[p.idx];
        rec.child_pid = p.pid;
        rec.entry = p.entry;
        rec.stack_top = p.stack_top;
        rec.regions = p.regions;
    });
    match sched::spawn_in_proc("spawned", proc_thread_entry, p.idx, p.pid) {
        Ok(tid) => {
            // SAFETY: single writer under IF=0.
            without_interrupts(|| unsafe {
                (*RECORDS.get())[p.idx].child_tid = tid;
            });
            Ok(p.pid)
        }
        Err(_) => {
            p.rollback();
            Err(STATUS_BUSY)
        }
    }
}

// ---- entry point 1: the syscall path (ADR-0019) ----------------------------

/// The creation sequence of ADR-0019, from the parent's syscall:
/// build a child process from registered image `img_id`, install the
/// attenuated inheritance list (spec: `(parent slot, rights)` pairs),
/// register the exit notification, hand the parent a `Process` handle,
/// and start the child's first thread at the image entry. Returns the
/// child's pid (the handler turns it into a positive status payload).
pub fn spawn_from(
    parent: u64,
    img_id: u32,
    inherit: &[(u64, u64)],
    notif: Option<(u32, u64)>,
) -> Result<u64, Status> {
    let half = prepare(img_id)?;

    // 4. Explicit handle inheritance: delegation-by-copy under the
    //    ADR-0015 attenuation rule — the source must hold COPY, and any
    //    rights amplification is refused loudly (never clamped).
    for &(slot, rights) in inherit {
        let inherited = (|| -> Result<(), &'static str> {
            if slot >= cap::CAP_SLOTS as u64 {
                return Err("inherit: source slot out of bounds");
            }
            let src = cap::read(parent, slot as usize)?;
            if src.rights & cap::RIGHTS_COPY == 0 {
                return Err("inherit: source cap lacks COPY");
            }
            if (rights as u32) & !src.rights != 0 {
                return Err("inherit: rights amplification refused");
            }
            cap::grant(
                half.pid,
                Cap {
                    obj: src.obj,
                    rights: rights as u32,
                },
            )?;
            Ok(())
        })();
        if let Err(e) = inherited {
            error!("spawn", "inheritance refused: {e}");
            half.rollback();
            return Err(STATUS_BAD_ARG);
        }
    }

    // 5. The parent's Process handle (READ|DESTROY — DESTROY is the
    //    forward-looking right: the rollback below needs it, and
    //    user-driven child reaping must not re-mint handles).
    let handle_slot = match cap::grant(
        parent,
        Cap {
            obj: CapObj::Process { pid: half.pid },
            rights: cap::RIGHTS_READ | cap::RIGHTS_DESTROY,
        },
    ) {
        Ok(slot) => slot,
        Err(_) => {
            half.rollback();
            return Err(STATUS_BUSY); // parent space full — no handle, no child
        }
    };

    // 6. Notification registration + record + first thread.
    match finish(&half, notif) {
        Ok(pid) => Ok(pid),
        Err(status) => {
            // finish rolled the child back; the handle in the PARENT's
            // space still needs undoing (it carries DESTROY exactly for
            // this).
            if let Err(e) = cap::destroy(parent, handle_slot) {
                error!("spawn", "rollback: handle destroy failed: {e}");
            }
            Err(status)
        }
    }
}

// ---- entry point 2: the boot path (ADR-0020) -------------------------------

/// Kernel-internal root spawn: the boot sequence creates the initial
/// service (the shell) with the same creation sequence as SYS_SPAWN —
/// but with no parent. `grants` are kernel literals placed into the
/// child's space in order (slot 0 first); no inheritance spec applies
/// (nothing exists above), and no Process handle is minted (the
/// bootstrap thread is not a process). Failure here is a boot failure:
/// the caller halts with the returned reason.
pub fn spawn_init(
    img_id: u32,
    grants: &[Cap],
    notif: Option<(u32, u64)>,
) -> Result<u64, &'static str> {
    let half = match prepare(img_id) {
        Ok(h) => h,
        Err(status) => {
            error!(
                "spawn",
                "spawn_init: image {img_id} preparation failed ({status})"
            );
            return Err("spawn_init: image preparation failed");
        }
    };
    for &g in grants {
        if let Err(e) = cap::grant(half.pid, g) {
            half.rollback();
            return Err(e);
        }
    }
    match finish(&half, notif) {
        Ok(pid) => Ok(pid),
        Err(status) => {
            // finish already rolled the child back.
            error!("spawn", "spawn_init: shell start failed ({status})");
            Err("spawn_init: initial thread could not be started")
        }
    }
}

/// The spawned child's first kernel-side moment: read its start facts
/// from the spawn record, register its user regions, and hand the
/// machine to the image's `e_entry` (ADR-0019).
fn proc_thread_entry(arg: usize) {
    // SAFETY: single reader under IF=0; the record was filled before
    // spawn_in_proc enqueued this thread, and records outlive the child
    // (explicit forget only, at teardown).
    let (regions, entry, stack_top) = without_interrupts(|| unsafe {
        let r = &(*RECORDS.get())[arg];
        (r.regions, r.entry, r.stack_top)
    });
    without_interrupts(|| {
        sched::set_current_user_regions(&regions).expect("spawned regions rejected");
        // SAFETY: the image pages and the stack page are mapped U/S in
        // this process's address space by prepare() (the scheduler put
        // us on its CR3 at switch-in); entry is the validated image
        // entry inside the registered regions; stack_top is the top of
        // the derived stack page; RSP0/scratch describe this thread
        // (programmed at switch-in, re-checked by enter_user).
        unsafe { syscall::enter_user(entry, stack_top) };
    });
}
