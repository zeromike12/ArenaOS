# ADR-0106: Native process-owned user threads

Status: Accepted; ring-3 thread/TLS guest proof and 20/20 T3 preservation passed.
Date: 2026-10-07

## Problem

The scheduler already saves a kernel context, process root, and FS.base for
each scheduler thread. Production processes still start with one ring-3 thread
and have no native create/join lifecycle. The 80-entry syscall pointer map is
already process-owned, so every thread in a process can validate the same
present mappings. Phase-12 TLS likewise already restores FS.base at scheduler
switch-in. These foundations permit threads to share one process without
copying its capabilities or creating a second process.

## Decision

Add a native user-thread ABI using the existing scheduler slots and exact
process address space:

- `SYS_THREAD_CREATE` accepts an executable user entry, one machine-word
  argument, an exact held `VmRegion` capability slot plus stack bounds, and an
  FS.base value. The kernel validates the entry's present user PTE is
  executable and non-writable, the stack is fully committed RW/NX with an
  unmapped guard page immediately below it, and TLS is aligned, present,
  writable, and NX. The VM capability must belong to the calling Process.
- The new scheduler thread uses the same Process ID, PML4, process-owned user
  spans, and capability table as its creator. It receives a separate kernel
  stack and explicit user stack. Creation adds no capabilities and copies no
  capability table. A thread therefore gains no process rights by existing.
- The scheduler restores that thread's FS.base before its first user
  instruction and on every later switch-in. Ring-3 entry starts with the
  declared argument in RDI and clears unrelated general registers.
- `SYS_THREAD_JOIN` accepts only a live or exited user thread owned by the
  caller's Process. It blocks atomically while the target runs, returns its
  stable exit status, and reclaims the scheduler record. An unjoinable wait
  returns a bounded busy result instead of halting the machine.
- `SYS_THREAD_DETACH` ends join ownership for one of the caller's user
  threads. After it exits, the kernel releases the exact VM stack capability
  only after switching away from that stack. Process teardown remains the
  final cleanup owner if the Process exits first.
- Thread IDs are descriptive. Join and detach always resolve the ID against
  the calling thread's already-known Process; an ID cannot select another
  Process's thread. Threads are limited to four additional user threads per
  Process. The existing 64 global scheduler slots, 128 capability slots, and
  eight VM regions per Process remain unchanged pending measured pressure.

The native runtime allocates one guarded VM region per thread. Its first page
holds a small start record and unique TLS control block, the next page remains
uncommitted as a stack guard, and the remaining committed pages form the user
stack. The runtime's join handle owns the VM region until join; dropping it
requests detach and lets the kernel release that region after thread exit.

## Why process threads fit the existing capability model

All threads in one Process already resolve syscalls through the same exact
Process capability space. This is the native process-sharing contract, not
ambient inheritance into a child process: creating a thread neither copies
nor attenuates a capability. The thread ID only identifies one member of the
current Process when joining or detaching. It has no cross-process effect.

An exact `VmRegion` capability is required to identify the user stack, and
release refuses while any live thread uses that region. Entry and stack
addresses are descriptive values checked against current PTEs and W^X rules;
they do not grant mapping, execution, or process authority.

## Bounds and accounting

Each live user thread consumes one global scheduler slot, one 96 KiB kernel
stack, one bounded guarded VM reservation, and one unique FS.base TCB. The
initial bound is four user-created threads per Process and the scheduler's
existing 64 slots globally. The VM API remains bounded at eight regions per
Process and 128 globally; ordinary 16 MiB heaps are reserved lazily, so the
new per-thread stacks compete visibly with other VM reservations. Admission
refusal must leave the cap table, scheduler queue, VM mappings, and frame
count unchanged.

## Consequences

- Threads are real concurrent ring-3 execution contexts sharing one PML4 and
  Process capability space; they are not hidden child processes.
- The existing per-thread FS.base save/restore becomes the native TLS path for
  user-created threads.
- A thread stack cannot be released while its thread is live. Join and detach
  provide explicit lifecycle paths; process teardown kills live threads and
  reclaims their remaining mappings and kernel stacks.
- The Process owns one exit status. It is recorded only when its final live
  thread exits; per-thread status remains available until that thread is
  joined.
- Mutexes and condition waits are not defined here. If a process-scoped
  wait/wake object is needed for efficient synchronization, a separate ADR
  must explain why the existing notification, IPC, and timer bounds do not
  fit and must use exact capability handles rather than POSIX futex behavior.

## Validation required

The installed APB1 guest must create at least two and several concurrent
ring-3 threads, prove unique FS.base/TLS values, increment shared heap state,
read a shared read-only mapping, join stable exit statuses, refuse stale or
cross-Process IDs, refuse stack release while live, reclaim stack mappings
after join/detach, and return Process/thread/cap/VM accounting to baseline.
Process death with live user threads must kill them and reclaim all process
resources without leaving stale join state.
