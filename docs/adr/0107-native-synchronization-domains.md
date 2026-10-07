# ADR-0107: Native synchronization domains

## Status

Accepted and implemented for Phase 13.

## Context

ADR-0106 adds concurrent ring-3 threads sharing one Process address space and
capability table. The userspace runtime needs a Mutex, a multi-waiter
Condvar, and Once with blocking that does not spin while another thread owns
the CPU.

The existing mechanisms do not provide that contract:

- An ArenaOS Notification merges wake badges but holds at most one parked
  waiter. It is appropriate for service/device events, not a condition with
  several waiting application threads.
- Application code does not receive notification-mint authority. Only the
  Desktop's private factory can create owner notifications. The 64-entry
  notification table is already budgeted for kernel services and private
  application clocks; allocating one notification per blocked thread would
  consume that shared budget and compete with established uses.
- Timers are one-shot deliveries to a held Notification. They do not compare
  a condition generation atomically with parking and cannot wake one of
  several waiters on a condition.
- IPC endpoints are synchronous process rendezvous with one server and a
  bounded caller queue. Using them between threads in one process adds
  message copying and process-facing authority to a local synchronization
  operation.

Yielding spin loops can implement mutual exclusion but consume CPU while a
thread waits and do not provide a useful condition wait. A small native wait
mechanism is therefore required.

## Decision

Add a bounded `SyncDomain` object minted only through a Desktop-held
`SyncDomainFactory`. An installed app's signed `FLAG_NATIVE_SYNC` is a request;
the manager still applies its launch policy, creates a domain only for an
approved app that requests it, retains the owning capability, and explicitly
grants READ|WRITE to each native Process that should share that instance's
synchronization state. Built-in applications do not request a domain. The
child cannot destroy, copy, or delegate the domain. Helpers receive it only
when the manager includes that exact grant. Domain identity and condition
keys do not authorize operations by themselves; every operation also
requires the held domain capability.

The kernel owns generation-checked wait keys inside each domain. A key has a
monotonic sequence and a bounded waiter set. Native calls provide the domain
cap slot, descriptive key token, and observed sequence. Waiting compares the
sequence and parks the calling scheduler thread atomically; signaling advances
the sequence and wakes one waiter or all waiters. A signal before the waiter
parks changes the sequence, so the later wait returns immediately instead of
losing the wakeup. Timed waits use a relative monotonic deadline and the
existing 100 Hz tick; timeout precision is therefore up to one tick late.

Initial bounds:

- 64 live domains, matching the existing Process-record bound;
- 32 live keys per domain and 2,048 keys globally;
- 64 wait records globally, matching the scheduler-thread bound;
- at most one active wait per scheduler thread.

Keys are explicitly created and destroyed through the exact domain cap, and
destruction is restricted to the Process that created the key. Tokens include
a generation and slot; stale or cross-domain tokens refuse. Destroying a key
with a parked waiter returns BUSY. Process teardown retires every key created
by that Process, decrements domain occupancy, and wakes any delegated waiter
with a typed service-gone result. This bounds helper-local key lifetime even
when the AppInstance domain remains live. Destroying the owning
domain invalidates all its keys and wakes its parked waiters with a typed
refusal. Process teardown removes wait records belonging to that Process before
killing its threads. Wait registration and `try_block_current` execute under
the syscall SFMASK IF=0 contract; the wait syscall refuses an IF-enabled
caller, preserving the atomic sequence-check/register/park boundary.
Domain-owner teardown invalidates domains and wakes waiters even if a
delegated holder is still alive.

The signed APB1 `FLAG_NATIVE_SYNC` bit requests a domain at launch; it is
descriptive metadata and does not mint or convey a capability. The trusted
manager validates the installed app policy before it creates the domain and
inserts the exact SyncDomain descriptor into Startup ABI v2. Existing built-in
applications retain their pre-Phase-13 startup cap inventory and do not receive
an unused synchronization cap.

The additive ABI calls are domain create (68), key create/destroy (69–70),
sequence read (71), wait (72), wake-one/wake-all (73), and domain accounting
(74). Timeout returns the native `STATUS_TIMEOUT` (-8). The wait domain is
bounded at 64 domains, 32 keys per domain/2,048 global keys, and 64 global
waiters; waits over 24 hours refuse. They are ArenaOS-native operations; no Linux
futex interface or POSIX semantics are introduced. Mutex uses an atomic lock
word plus its private wait key. Condvar uses the sequence-and-park contract
while releasing and reacquiring its associated Mutex. Once uses a one-time
state and wake-all when initialization completes. Runtime guards cannot be
sent to another user thread. Abrupt Process teardown remains the recovery
boundary for a thread that bypasses safe Rust cleanup.

## Consequences

- A Condvar can support multiple concurrent waiters without changing
  Notification's single-waiter contract or consuming one Notification per
  thread.
- The wait domain is an explicit capability. App IDs, Process IDs, key tokens,
  and user pointers carry no synchronization authority.
- For an app approved for `FLAG_NATIVE_SYNC`, the manager retains one owner
  cap for its AppInstance and retires it after the AppInstance's ProcessGroup.
  Apps that do not request synchronization consume no domain or cap slot.
  This keeps Phase-12's 32-session cap budget intact while preserving the
  exact 128-slot table and slot-127 APB1 accounting. Kernel teardown sweeps
  domains owned by a dying manager, and Process teardown clears its parked
  wait records.
- Wait keys use fixed tables, bounded scan cost, and observable domain/key/
  waiter occupancy. The Phase-12 capability width and low-32 accounting stay
  unchanged. ABI-v2 gains one exact SyncDomain descriptor and one inherited
  grant; the bounded ABI-v2 descriptor/inheritance ceilings increase by one
  to seven descriptors and eight total caps including startup slot 0.

## T1 guest proof

The signed installed-app guest proves four contending ring-3 threads,
notify-one and notify-all across four Condvar waiters, Once contention, a timed
wait, sequence-change-before-wait, invalid cap/key refusal, and zero key
occupancy after RAII destruction. A signed helper is terminated while its
ring-3 worker is parked; the parent observes its wait record and two
creator-owned keys reclaimed. The registry guest also passes its six-window
resource oracle and returns to its teardown baseline.

## Remaining validation

The mixed-load fixture must record domain, key, and waiter high-water counts.
The built-in application startup profile now validates the new exact
SyncDomain descriptor while keeping its kind-specific optional tail checks
intact. The first two attempted T3 runs failed on that profile mismatch and
are not counted as green. After the focused M10 Desktop launch regression
passed, the corrected EFI passed `tools/stability_loop.sh 20` with 20/20 fresh
boots, zero failures, and full historical/network/graphical lifecycle proofs.
The EFI SHA-256 was
`9500b0d8bd11e35a522cc9f64fa39ed1229bfbadc28b3a4aae101eb03dab5e3a`. The full
final qualification remains required at source freeze.
