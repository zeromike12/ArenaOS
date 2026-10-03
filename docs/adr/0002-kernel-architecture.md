# ADR-0002: Kernel architecture — capability-based hybrid microkernel

Status: Accepted
Date: 2026-09-25
Milestone context: Phase 0 — Architecture

## Problem

Choose the kernel's structural architecture before writing kernel mechanisms.
This determines where drivers, filesystems, and policy live; what the syscall
surface means; how failures propagate; and how the security model is
expressed. It is effectively irreversible past Phase 3.

## Options considered

**Monolithic** (Linux, FreeBSD): all services in one kernel address space.
Best-known performance profile and simplest early momentum. Fatal for our
goals: driver bugs are kernel bugs; sandboxing and fine-grained permissions
become advisory layers (LSM-style) bolted onto ambient authority; and the
codebase grows faster than a small team can audit. Rejected.

**Classic microkernel** (L4 family, seL4, MINIX 3): kernel = address spaces,
threads, IPC. Maximum isolation, minimum kernel. Historic weakness: IPC cost
on *every* critical path (every disk read, every packet), and page-fault or
timer policy pushed out of the kernel hurts latency. seL4-level minimality
targets formal verification we are not staffed for. Rejected as-is, adopted
heavily in spirit.

**Hybrid** (Windows NT, XNU): microkernel-flavored objects, but most services
run in kernel mode for performance. This is the "have it both ways" answer —
and pays both bills: kernel-sized attack surface *and* microkernel IPC
complexity. Rejected as a *justification*, though the word "hybrid" survives
in our naming for a different reason (below).

**Unikernel / exokernel:** single-address-space appliances (MirageOS) or
minimal-multiplexing kernels (MIT exokernel). Wrong shape entirely for a
general-purpose multi-application desktop OS; but exokernel's "applications
manage their own resources over untyped hardware abstractions" directly
inspires our untyped-memory objects. Rejected as architecture, mined for
ideas.

## Decision

**A capability-based hybrid microkernel:**

1. Kernel implements mechanisms only: physical/virtual memory, threads,
   scheduling, IPC, capability spaces and rights checking, interrupt
   dispatch, time. Target: a kernel a small team can hold in its head.
2. Drivers, filesystems, network stacks, and all policy are userspace
   servers, isolated and restartable.
3. Every kernel object is referenced only through capabilities with explicit,
   attenuable rights. Syscalls are capability invocations (ADR-0006).
4. "Hybrid" pragmatism: performance-critical mechanisms stay in-kernel
   (notably VM fault handling and scheduling), and a minimal polled console
   stays in-kernel for panic diagnostics. We do not push page-fault handling
   or timers into userspace servers.
5. IPC is designed from day one as: synchronous rendezvous for control,
   badged notifications for events/interrupts, shared-memory rings for bulk
   data (ARCHITECTURE §7). No IPC design work is deferred to "when we need
   performance".

## Reasoning

- Our differentiators (VISION) — driver isolation, recoverable services,
  application sandboxing, explicit permissions — are *structural* properties.
  They exist in a capability microkernel by construction; in a monolith they
  are permanently aspirational.
- The classic microkernel performance objection is a 1990s result about
  *unoptimized* IPC on *old* hardware; L4-lineage kernels and Zircon/Fuchsia
  demonstrated that careful rendezvous + shared memory is within a small
  factor of monolithic throughput for real workloads. We accept the
  engineering obligation this creates and schedule it (M4 IPC foundations
  include benchmark harness, not an afterthought).
- Security: capabilities make the "confused deputy" class of bugs
  architecturally rare — the deputy can only act on what it holds.

## Downsides accepted

- More upfront design discipline: capability spaces, rights, and IPC buffers
  must be right before userspace can grow. We pay this in Phases 3–4.
- Driver development requires the IPC and resource-grant machinery to exist
  first (Phase 6 cannot start early).
- Cross-address-space debugging is harder; mitigated by structured logging
  everywhere and, later, a kernel introspection interface that is itself
  capability-gated.
- We will be slower than Linux at some things forever. Accepted: we are not
  building Linux.

## Future implications

- The kernel ABI (ADR-0006) must stay small; feature pressure routes to
  userspace protocols instead.
- SMP (Phase 3+) must be designed into capability/IPC locking from the start
  even while we run single-core — data structures are lock-ready, tests are
  deterministic.
- If IPC benchmarks (M4 exit criteria) show unacceptable control-path latency,
  the remedy is fast-path tuning (direct register transfer, batching), *not*
  moving servers into the kernel; that would require superseding this ADR.
