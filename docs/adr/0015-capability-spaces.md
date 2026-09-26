# ADR-0015: Capability spaces — slots, rights, and the first gated invokes

Status: accepted (M3.4, 2026-09)

## Problem

ARCHITECTURE §2 commits ArenaOS to a capability-based object model: *the
only way to reference any kernel object is a capability stored in a
process's capability space, carrying explicit rights; syscalls are
capability invocations.* Milestone 3.3 built the address-space half of
the process object (`proc.rs`, ADR-0014); the roadmap's 3.4 demands the
capability-space half in minimal form:

* **Slots**: a bounded, per-process table of capability entries.
* **Rights**: an explicit permission mask carried by every capability,
  checked on every use.
* **copy / move / destroy**: the reference-management operations, with
  the security invariant that delegation can only *attenuate* rights —
  never amplify them.
* **Kernel-internal use only at first**: no syscall surface yet (that is
  M4's IPC/loader work), but the machinery must be *real* — capabilities
  must actually gate real kernel actions on real objects, not decorate
  data structures. A rights check nothing enforces is fake functionality.

Constraints inherited from earlier decisions: no heap-backed objects yet
(fixed tables only — ADR-0007/0012/0014 discipline), single writer
under IF=0 for all table mutation (ADR-0010/0013), exact frame
accounting in the M3 suites, W^X on every mapping we create (ADR-0008),
and the regression contract (ADR-0005): the m3 suite grows, never
shrinks.

## Options considered

**A. Fat pointer capabilities.** A capability is a kernel-heap pointer
to an object header plus a rights word, handed out as an opaque token
(L4/traditional-microkernel style). Rejected for now: we have no
kernel-heap object model with refcounts yet, pointer-validity questions
(use-after-free, forgery by guessing) demand a real object lifetime
story first, and the M3 kernel deliberately has zero dynamically sized
objects. This shape remains the likely M4+ evolution *underneath* the
slot tables (a cap slot can later hold an index into a registered
object table).

**B. Per-process slot tables over the existing fixed process table.**
A capability = `(process, slot index)`; the slot holds a small Copy
struct `(object reference, rights)`. All validation is index-based at
invoke time: the slot must be occupied, the rights must cover the
operation, and the *referenced object* must still be alive (checked
against the owning subsystem's live table — `proc::pml4_of` for process
caps). Unforgeable by construction while the only writers are kernel
paths: userspace (when it arrives in M4) will only ever pass slot
indices, never addresses or object identities. Chosen.

**C. A single global capability namespace** (one table, space ids as
part of the index). Rejected: per-process spaces are the architectural
commitment (isolation boundary, future revocation domains), and a
global table recreates the ambient-authority model capabilities exist
to eliminate.

## Decision

`kernel/kernel/src/cap.rs`, embedded in the `Process` struct
(`Process.caps`), so a capability space is born with `proc::create` and
dies with `proc::destroy` — the ADR-0014 promise that the process
struct is the anchor both halves attach to.

* **Shape**: `CapSpace` = fixed array of `CAP_SLOTS = 16` slots +
  occupancy count. A slot holds `Cap { obj: CapObj, rights: u32 }`;
  `CapObj` is `None | Process { pid } | Memory { phys, pages }` — the
  two object kinds that exist as real kernel objects today (address
  spaces from 3.3; untyped-style physical memory ranges as the seed of
  the ARCHITECTURE §4 untyped-memory ABI).
* **Rights** (u32 bits): `READ` (inspect what the cap references),
  `WRITE` (mutate the referenced object through an invoke), `COPY`
  (delegate: required *on the source* to copy/move it), `DESTROY`
  (remove the cap from its slot). Kernel-internal `grant` is the root
  of trust: it may install any object with any rights (this is the
  boot/root-task policy path; user-facing grant semantics arrive with
  M4 endpoints).
* **Delegation is attenuation-only**: `copy`/`move` take an explicit
  rights mask which must be a *subset* of the source's (requesting more
  is an error, not a silent clamp — refusals must be loud, ADR-0005),
  and the source must hold `COPY`. `move` = copy + source clear,
  atomically under IF=0.
* **Destroy removes the reference, not the object**: a capability is a
  *name* for an object; object lifetime stays with its owning subsystem
  (`proc::destroy` frees the address space; frames return through
  `frames::free`). Last-cap-deletes-object refcounting is explicitly
  deferred (Future implications). Consequently caps can dangle: every
  invoke re-validates the referenced object against its live table, so
  a cap to a destroyed process fails cleanly instead of touching freed
  memory.
* **Two real gated invokes** (the "kernel-internal use" the roadmap
  requires — each performs an actual kernel action, gated by actual
  rights checks):
  * `process_root(space_pid, slot)` — requires `READ` on a live
    `Process` cap; returns the target's PML4 root PHYS (the
    information-flow gate: no root address without a right to it).
  * `map_memory(space_pid, mem_slot, proc_slot, va, writable, exec)` —
    requires `WRITE` on a `Memory` cap *and* `WRITE` on a live
    `Process` cap in the same space; maps the cap's physical pages into
    the target's user half through `paging::map_user_page_4k` (the
    mutation gate: the untyped-memory → address-space binding of
    ARCHITECTURE §4, in miniature, with W^X flags per ADR-0008).
* **All operations** run under `without_interrupts` (single-writer
  discipline), validate slot bounds and occupancy first, and return
  `Result<_, &'static str>` with distinct, testable refusal strings.

## Reasoning

* **Matches the commitment, minimally.** Slots/rights/copy/move/destroy
  are exactly the roadmap line; the two invokes make rights *mean*
  something today (a WRITE-less cap genuinely cannot alter an address
  space), which is the difference between a capability mechanism and a
  struct with a permission field nobody reads.
* **No new lifetime machinery.** Everything lives inside the existing
  fixed process table — zero allocation, zero refcounting, exact
  accounting preserved (a cap describes frames; it never owns them —
  the test frees them through the normal path).
* **Unforgeability now, syscall-readiness later.** Userspace will pass
  `(slot)` indices into a space the kernel already selected from the
  calling thread's process; every validation rule needed for that
  surface (bounds, occupancy, rights subset, object liveness) is
  implemented and tested kernel-internally first.
* **Loud refusals** (subset violation = error, dangling = error,
  occupied slot = error) keep the suite's measure-don't-trust style:
  each refusal is an asserted `Err`, each success an observed effect
  (a mapped page read back under the target's CR3).

## Downsides accepted

* **16 fixed slots per process, 32 processes** — small by real-system
  standards; fine for kernel-internal use and the suites, revisited
  when userspace cspaces need growth (M4: likely a paged cspace or
  heap-backed table behind the same API).
* **No revocation trees / no badges / no minting**: copy attenuation is
  the only rights transformation; recursive revocation of derived caps
  needs derivation tracking (a parent field or generation counters) —
  deliberately deferred until a real delegating userspace exists.
* **Dangling caps persist** until destroyed (destroy-the-reference
  semantics). Every invoke pays a liveness re-check; acceptable at this
  scale and the price of not inventing refcounts prematurely.
* **`Memory` caps trust the granter** that `(phys, pages)` describes
  frames it owns — fine while granters are kernel code and tests; the
  untyped-object story (kernel-allocated, kernel-tracked) replaces this
  when grants become user-triggered.

## Future implications

* **M4 syscall surface**: `SYS_cap_copy/move/destroy` + invoke-style
  calls take slot indices against the *calling thread's process* space;
  the validation core is already written — the syscall layer adds
  argument unmarshalling and nothing else.
* **Object growth**: endpoints (IPC, M4.4), thread caps, and untyped
  memory objects extend `CapObj`; the slot/rights/attenuation machinery
  is object-kind-agnostic by construction.
* **Last-cap destruction** (refcounted objects deleted when their final
  capability dies) becomes mandatory the moment a cap can be the only
  reference to a heap-backed object; the invoke-time liveness checks are
  the seam where that plugs in.
* **Revocation**: derivation tracking (copy records parent slot +
  generation) enables recursive revoke; would amend this ADR.
* **Root task** (4.5) is born with a granted initial set (its process
  cap, its memory caps, the boot endpoint) — `grant` is exactly that
  boot-policy primitive.

Milestone context: M3.4 — completes Milestone 3's object-model
foundation (threads ADR-0012/0013, address spaces ADR-0014, capability
spaces here) before M4 puts userspace on top of it.
