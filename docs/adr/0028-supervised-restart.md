# ADR-0028 — Supervised restart: a dead service is an answer, and then a new service

*Milestone 6.5. Status: accepted.*

## Problem

ADR-0022 put drivers in ring 3 and justified it plainly: a broken
driver must not be able to take the kernel with it. Four drivers
later that claim is well tested — storaged, netd, rngd, inputd, and
consoled all fail in ways that stop at the process boundary.

But isolation is only half a promise. "The kernel survives your
driver" is worth very little to the program that was *using* the
driver. Before this milestone, if a service died:

- a client blocked in `SYS_IPC_CALL` waited **forever**. It has no
  timeout, no liveness check, and no way to observe the server at
  all; the reply it was parked for simply never came.
- the service never came back. Nothing watched it, and nothing could
  have restarted it if it had.
- the corpse could not even be fully destroyed: `proc::destroy` tore
  down the address space but left any parked thread live, counted by
  every drain and reaped by nothing.

The third point is the sharpest. `State::Blocked` has carried the
comment "counts as live until woken or (later milestones) killed"
since M3.1. A supervisor cannot restart a driver it cannot kill, and
a driver worth restarting is almost always parked — blocked in
`SYS_IPC_RECV` waiting for work, or in `SYS_WAIT` for an interrupt
that will never arrive because the device is wedged.

## Approaches considered

**A. Timeouts in clients.** Every client learns to give up. Rejected:
it pushes a kernel fact (the server is gone) into a guess each client
must make separately, it needs a timer facility ring 3 does not have,
and a correct timeout is unknowable — a slow disk and a dead driver
look identical from the outside.

**B. Endpoints die with their server.** Clients' capabilities would
fault on use, which is at least prompt. Rejected: it destroys the
thing a restart needs. A client's capability names the endpoint; if
the endpoint dies, every client must be re-granted a new one, which
means every client must be found, which means the supervisor must
know the whole topology. Keeping the endpoint is what makes restart
invisible to clients.

**C. Kernel answers for the dead server; endpoint survives; a
supervisor respawns the image with the same grants.** Chosen.

**D. A ring-3 supervisor (a real init).** The microkernel-shaped
answer, and probably where this ends up. Deferred deliberately, see
below.

## Decision

### 1. A dead server produces a typed status

`STATUS_SERVICE_GONE` (-5). When a process is destroyed,
`ipc::fail_calls_for_server` walks the endpoints it served and fails
every call in flight: `Delivered` (in its hands) and `Waiting`
(addressed to it) slots become `Failed`, and their callers are woken
to receive the status instead of a reply. `Replied` slots are left
alone — that answer is real and the caller is entitled to it even
though the server has since died.

Which endpoints a process served is decided **by capability**: they
are exactly the ones it holds `Endpoint` + READ for. No registry to
keep in step with reality, and no way for the two to disagree,
because the capability *is* the authority to serve.

The status deliberately says nothing about whether the request was
performed. The kernel cannot know — the server may have completed the
work and died before replying — and a status that implied otherwise
would be a lie. Clients with non-idempotent operations must treat it
as *unknown*.

### 2. A blocked process can be killed

`sched::kill_threads_of` zombies every live thread of a process
without running another instruction in it: parked threads never
return from the syscall they are in, so nothing resumes on their
kernel stacks and the ordinary reaper reclaims them with the usual
canary check. `plan_switch` now skips stale ready-ring entries rather
than performing surgery on the ring.

Ordering is load-bearing: `ipc::release_blocked_of` drops every
kernel reference to those threads — endpoint server slots,
notification waiters — **before** they are killed. Waking a corpse is
a halting offence in this kernel, and that rule is correct, so the
ids must be gone before the threads are.

### 3. The supervisor

`kernel/src/supervise.rs`. A supervised service is a registry image
plus the exact grant list it was spawned with. Restarting means: reap
the corpse's spawn record, replay the same capabilities, spawn the
image again. "The same grants" is not a reconstruction — an `Mmio`
window is a physical base and a page count, an `Endpoint` is an index
— it is the identical list, replayed.

Deaths are **noticed** in `proc::destroy` and **acted on** in
`supervise::poll`. Restarting allocates frames and maps pages, and
destroy's context must do neither; this is the same notice/act split
ADR-0027 uses for the console mirror's wake.

`MAX_RESTARTS` (3) bounds it. A driver that dies three times is not
going to be fixed by a fourth spawn, and an unbounded supervisor
turns a broken driver into a machine that does nothing but restart
it. Giving up is logged loudly and leaves the service **offline** —
the same honest state the machine is in when the device is absent,
and one every client already handles.

### 4. The spawn-record GC debt, closed

ADR-0025 noted that spawn records are a bounded table and that
production spawns never retire theirs. Restarts make that fatal: one
record per death until spawning becomes impossible. The supervisor
now reaps the corpse's record before spawning the replacement, and
the restart test asserts the record count is **flat** across a full
cycle — it would be +1 without the reap.

## Reasoning

Every piece here follows from one idea: *the failure of a service
should be a fact the system states, not a silence the system leaves*.
A hung client, a thread that cannot be killed, and a service that
never returns are all the same bug wearing different clothes — the
machine knowing something and not saying it.

Keeping the endpoint alive is what makes restart cheap. Clients never
learn a pid, so a new instance behind the same endpoint is not an
event they have to handle beyond retrying the one call that failed.

## Downsides accepted

- **No state recovery.** A restarted driver starts from nothing. A
  client halfway through a multi-request transaction must cope, and
  the kernel offers it no help beyond the typed status. Real
  protocols will need session identifiers or idempotent operations;
  that is a later milestone with its own evidence.
- **No back-off and no dependency ordering.** Restarts are immediate
  and independent. A service whose dependency is still restarting may
  fail and burn a restart credit.
- **`poll` must be called.** Deaths are marked, not acted on, so
  something must run `supervise::poll` at thread context. The suite
  calls it directly. Wiring it into the boot/idle path for the
  resident drivers is the obvious next step and is deliberately not
  claimed here — a supervisor nobody runs would be exactly the kind
  of half-built claim this project avoids.
- **The supervisor is in the kernel.** Minting an `Mmio` capability
  is the one authority the kernel has never delegated (ADR-0021), and
  a supervisor that cannot re-grant a device window cannot restart a
  driver. Putting supervision in ring 3 means exporting that
  authority — a new syscall and a new trust boundary that deserve
  their own decision rather than being smuggled in as a detail of
  "restart a driver".

## Amendment (M7.1b): calls sent AFTER the server died

This ADR answered "what happens to a caller that was in flight when
its server died" and, without noticing, left the neighbouring case
open: a call sent *afterwards*. It queued on an endpoint nobody would
ever read and blocked forever — the same hang this decision existed to
remove, one instant later.

It stayed invisible because the suites always destroyed a server while
somebody was mid-call. The first thing to hit it in earnest was the
network stack, and unavoidably so: a stack discovers its driver is
gone **by calling it**.

An endpoint whose serving process is destroyed is now marked
**orphaned**, and calls to it are refused immediately with
`STATUS_SERVICE_GONE`. The flag clears when any process takes up the
serve side again — which a restarted service does by its first
`recv`, so there is no registration step to forget and no way for the
flag to outlive the situation it names.

The general lesson is worth more than the fix: "the service is gone"
has to be answerable at *every* point a client can touch it, not only
at the instant of death. A typed error that covers one of two paths
leaves the other path exactly as broken as it was, and looks tested.

## Future implications

- The obvious next question is D: a ring-3 supervisor. The shape of
  this module — a table of (image, grants, pid) and a `poll` — was
  chosen to be portable to that world, where `register` becomes a
  syscall and grants come from a capability the supervisor already
  holds rather than from kernel literals.
- `STATUS_SERVICE_GONE` is now part of the client ABI. Every service
  client written from here on should treat it as "retry", and the
  drivers' own clients (blktest, fstest, nettest, …) are the natural
  place to prove that once restart is wired into production.
- Killing blocked threads makes a `kill` shell command possible for
  the first time, and with it in-guest fault injection a user can
  drive. That is a small, honest addition once there is a capability
  to gate it on.
- Fault injection has exactly one image pair today (`faultd` /
  `faulttest`) and it is deliberately trivial: killing storaged to
  watch what happens would prove the same thing while risking the
  filesystem. When restart is wired into production, killing a real
  driver mid-I/O becomes the next test, and the crash-consistency
  work of ADR-0023 is what makes that survivable.
