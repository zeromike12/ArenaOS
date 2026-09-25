# ADR-0006: Kernel/user ABI philosophy — capability invocation

Status: Accepted (direction). Register-level specification is due as its own
ADR at Milestone 4 when the first syscall is implemented.
Date: 2026-09-25
Milestone context: Phase 0 — Architecture

## Problem

Fix the *shape* of the kernel/user interface before any kernel object exists,
because every later API decision (processes, IPC, files, permissions) is
expressed through it, and because ABI stability is a 20-year commitment.

## Options considered

**Unix-style syscall table** (numbers + fd integers + errno): rejected
wholesale. File descriptors are ambient, untyped, race-prone (the `ioctl`
everything-bag), and inheritability via `fork` makes rights flow implicitly —
incompatible with the capability model (ADR-0002) and with explicit
permissions (VISION).

**COM/CORBA-style object brokers:** vtables in shared memory or marshaled
object references: powerful, but puts policy in the ABI and makes the kernel
interface unbounded. Rejected.

**seL4-style pure capability invocation:** one syscall (`Call`) plus a handful
of object-specific methods, arguments packed into message registers. The
closest match to our model and proven at scale (seL4, Zircon's handle+syscall
pair). Adopted as the shape, with our own ergonomics decisions.

**Zircon-style split:** small fixed syscall set + typed handles with rights
(`ZX_RIGHT_*`), objects created from handles to other objects. Also adopted in
spirit — rights bits per object type are exactly the "explicit permissions"
mechanism.

## Decision

The ArenaOS kernel ABI is:

1. **Handles are capability slots.** A process's handle indexes its own
   capability space; each capability = (object reference, rights mask).
   Handles are meaningless outside their space; copying a capability into
   another space is an explicit, rights-attenuating operation.
2. **A small fixed syscall core**, target size ~25–35 calls, grouped as:
   - capability space ops: move, copy (with attenuation), destroy, mint
   - object ops per kernel type: thread, address space, endpoint (IPC port),
     notification object, untyped memory, interrupt port — each with a
     create-from-parent-capability pattern
   - IPC: `call` (sync rendezvous), `reply`, `notify` (async badged),
     `wait`
   - execution: thread start/resume/suspend, address space map/unmap
   - system: time (monotonic + wall, capability-gated for wall-clock)
3. **Typed status codes**: every call returns a structured status
   (domain, code) — no errno namespace, no exception-like restart semantics.
4. **Bulk data never flows through syscall registers**: message registers
   carry small structured arguments and *capability transfer slots*; payload
   moves through shared memory established via untyped/VMAR mapping
   (ARCHITECTURE §7).
5. **Versioning**: the ABI version is negotiated at process start (recorded in
   the boot/handshake); breaking changes require a new version and a
   superseding ADR. Userspace service protocols are *not* part of the kernel
   ABI and evolve independently.
6. Register-level convention (which registers, how many message words, how
   capability transfer slots are addressed) is deferred to the M4 ADR — it
   depends on scheduler and IPC implementation realities we will only know
   then. This ADR fixes the semantics, not the encoding.

## Reasoning

Capability invocation is the only shape consistent with "no ambient authority"
(ADR-0002/0004). Splitting *kernel ABI* (tiny, frozen, audited) from
*service protocols* (rich, evolving, userspace) keeps the auditable surface
small while letting the ecosystem move fast. Both seL4 and Zircon validate the
family; our contribution is refusing POSIX ergonomics on top of it and
designing permission manifests (Phase 8) directly against rights masks.

## Downsides accepted

- Steeper learning curve for anyone expecting Unix: no `open()`/`read()`
  mental model. Accepted deliberately (VISION: modern APIs, not inherited
  ones); a userspace compat subsystem can translate later.
- Rights-mask design mistakes are expensive; we mitigate by keeping M4's
  first object set minimal (thread, address space, endpoint, untyped) and
  growing it only with use cases in hand.
- Capability transfer in messages needs careful atomicity design (the
  "capabilities in flight" problem) — scheduled explicitly in M4.

## Future implications

- Every kernel object added later must fit: created from parent capabilities,
  addressed by handles, rights-masked. No global namespaces may appear in the
  kernel ABI, ever.
- The userspace standard library will feel object-oriented
  (`endpoint.call(msg)`), not fd-oriented.
- Permission manifests (Phase 8) compile down to initial capability sets +
  rights attenuation rules — the ABI already supports exactly this.
