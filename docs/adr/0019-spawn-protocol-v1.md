# ADR-0019: Spawn protocol v1 — image capabilities, explicit handle inheritance, exit notifications

- Status: accepted
- Date: M4.5 (IPC v1 shipped; processes must now create processes)
- Relates to: ADR-0015 (capabilities), ADR-0016 (executable format + loader),
  ADR-0017 (syscall ABI v1), ADR-0018 (IPC v1), ARCHITECTURE §5 (process model),
  §7 (no global names — services reached by capability)

## Context

Through M4.4, processes are created by kernel test code: `proc::create` +
`elf::load` + hand-mapped stacks + `spawn_in_proc`. A desktop-class OS needs
process creation to be a *user-driven* service: a process holding the right
capabilities creates the next one, chooses exactly which handles the child
inherits, and learns when the child dies. The ROADMAP's M4.5 names the
target — "root task + spawn protocol: process creation from image
capabilities with explicit handle inheritance; supervisor restart demo" —
and ARCHITECTURE §7 forbids global names, so the image a child is built
from must itself be reached by capability. The seL4 end-state (a root task
minting every kernel object from untyped memory) needs user-driven kernel
object allocation, which does not exist yet; v1 takes the smallest honest
step: one kernel-mediated spawn syscall, driven by caps, with the
supervisor process acting as the root task of its own subtree.

## Decision

### Image capabilities and the registry

A new cap kind `CapObj::Image { img_id }` references a kernel-side image
registry (fixed table, v1 populated with exactly one entry: the embedded
rust-lld payload image — the same bytes the M4.1–M4.3 tests parse, load,
and run). `READ` on an image cap means "may spawn from it". When a
filesystem exists, images become files and the registry grows a
file-backed source; the cap shape does not change.

### `SYS_SPAWN` — registry slot 12

```text
RAX = 12
RDI = image cap slot                     (needs READ)
RSI = user ptr → inheritance spec        (array of (src_slot: u64, rights: u64))
RDX = spec entry count                   (0..=4)
R10 = notification cap slot or CAP_NONE  (needs WRITE when present)
R8  = exit badge (nonzero when R10 present)
→   RAX = child pid (positive payload), or a typed negative status
```

The kernel-side sequence, all under the caller's syscall (IF=0), with
**rollback on every partial failure** (a refused spawn costs zero frames
and leaves zero objects — the M4.1 double-load discipline, applied to
process creation):

1. Validate the image cap and look the bytes up in the registry; run the
   ADR-0016 validator on them (a corrupt registered image is refused at
   spawn time, not loaded).
2. `proc::create` the child; `elf::load` the image into it; map ONE stack
   page (RW+NX) at the first page above the image's top segment VA —
   placement derived from the image, never hardcoded.
3. **Explicit handle inheritance**: for each spec entry, read the source
   cap from the *parent's* space (must hold `COPY` — inheritance is
   delegation-by-copy under the ADR-0015 attenuation rule), refuse any
   rights amplification loudly, and `cap::grant` the attenuated copy into
   the child's space in spec order (first free slots).
4. Register the **exit notification**: when `R10` names a notification
   cap (WRITE — the parent lends the notify side) and a nonzero badge,
   the child's process records `(nid, badge)`.
5. Grant the parent a fresh `Process { pid }` cap for the child with
   `READ | DESTROY`: READ is the observational handle v1 uses; DESTROY
   is the forward-looking right — no user-facing destroy syscall exists
   yet, but the spawn rollback path needs it to undo a granted handle,
   and user-driven child reaping (named in Consequences) must not
   require re-minting handles. It lands in the parent's first free
   slot; the child's *pid* is the syscall's positive payload.
6. Start the child's first thread (`spawn_in_proc`) at the image's
   `e_entry` with user regions derived from the parsed segments + the
   stack page, and a spawn record carrying the start facts.

### Exit notification

When a thread exits (`SYS_THREAD_EXIT`) and it is the **last live thread
of a process with a registered exit notification**, the kernel fires
`ipc::notify(nid, badge)` before the scheduler takes the thread away.
The supervisor learns of the death through the ADR-0018 primitive it
already blocks on — no new waiting mechanism, and the badge survives
(merged) even if the supervisor is mid-spawn when the child dies. In v1
processes are single-threaded by construction, so "last live thread" is
exact; the multi-thread rule (notify on last-thread, not on each) is
already the implemented semantics.

### The spawn registry

A fixed table of records `{ parent, child_pid, child_tid, entry,
stack_top, regions }` — the child thread's entry trampoline reads its
start facts from its record, and the test side reads the table as
machine-state evidence (which children exist, what they were). Records
are released explicitly (`spawn::forget`) at teardown; v1 has no
automatic record GC because v1 has no child destroy-from-userspace yet.

### What v1 deliberately does NOT do

- **No user-driven destroy**: `proc::destroy` stays kernel-side. The
  supervisor of the demo holds `READ`-only Process caps; reaping dead
  children is the test's teardown. Destroy-from-cap is the natural next
  authority step (it needs a "no threads live" guard and a zombie-child
  policy — named in Consequences).
- **No bootinfo/argv page**: the child starts at `e_entry` with an empty
  stack. The embedded image's program is role-free (it writes its
  message and exits), which is exactly what the restart demo needs; a
  real init protocol (arguments, environment, an auxv-like bootinfo
  page) arrives with the shell milestone (M4.6) or the FS, whichever
  needs it first.
- **No image registry mutation from userspace**: registering images is
  kernel-side until a filesystem exists.
- **No multi-threaded children, no spawn-of-spawn restrictions**: any
  process holding an Image cap + COPY-able handles may spawn; policy
  (who MAY create processes at all) is the root task's job, and the
  root task is a *policy* milestone on top of this mechanism.

### The demo the milestone must prove (m4 test `spawn_restart`)

A supervisor process (hand-assembled ring-3 payload in its own address
space, holding an Image cap, a Notification cap, and one COPY-able
Memory cap) spawns the real payload image **twice in a row**: each
spawn passes an explicit inheritance spec (the Memory cap, attenuated),
the notification cap, and a badge; each time it then `wait`s for the
child's exit badge. The child is the untouched M4.3 program — it
verifies its META, stamps its bss, writes its pinned message through
`debug_write` (its message appears on the console **twice**: the
restart, visible), and exits with META's code 42. Kernel-side
assertions: both children exited 42 (via the spawn registry's thread
ids), the inherited cap landed in each child's first free slot with
exactly the attenuated rights, the parent's Process caps landed for
both children, the parent KEPT its original Memory cap (copy, not
move), dispatch/notification/wait/block counters exact, threads reaped,
and frame teardown exact across all three address spaces.

## Alternatives considered

1. **seL4-style root task from day one** (all kernel objects minted
   from untyped memory by the first userspace task) — the correct
   end-state, but it needs a retype/allocation syscall surface,
   scheduling contexts as objects, and a fault-handling protocol. Doing
   it now would stall the milestone chain (shell, FS) behind a large
   redesign; the v1 protocol's cap shapes (Image, Process handles,
   inheritance specs) are forward-compatible with it.
2. **fork/exec-style spawn (POSIX)** — rejected with the standing
   constraint: POSIX semantics are not a design target; an image cap +
   explicit inheritance list is simpler, auditable, and capability-native.
3. **Exit status via a blocking `proc_wait` syscall** — a second waiting
   primitive competing with notifications; the badge-on-death design
   reuses ADR-0018's tested blocking path and composes with the
   supervisor's other event sources (one `wait` can merge "child died"
   with "device ready" — the L4 lesson §7 already commits to).
4. **Child destroy implicit on last-thread exit** — frees frames eagerly,
   but destroys the child's cap space and address space before the
   supervisor could inspect exit state or inherited handles; explicit
   destroy (later, from a cap with DESTROY rights) keeps authority
   visible.

## Consequences

**Gains:** processes create processes from ring 3 through capabilities —
the first user-driven kernel service beyond IPC; explicit, attenuating
handle inheritance (no ambient authority, no global names); death
notification composible on the notification primitive; the supervisor
restart loop proven end to end with the real image running twice.

**Costs / deferred (each named):**

- Destroy-from-userspace + zombie-child policy (the supervisor cannot
  yet release a dead child's frames itself).
- Bootinfo/argv for spawned children (role-free images only until then).
- User-driven image registration (filesystem milestone).
- Spawn policy (who may spawn at all) — root-task *policy* on top of
  this mechanism; v1 grants the mechanism to any Image-cap holder.
- The registry's fixed capacities (images: 4, records: 8, inheritance:
  4 entries) — bounded refusals (`STATUS_BUSY`/`STATUS_BAD_ARG`), not
  silent growth, consistent with the house discipline.
