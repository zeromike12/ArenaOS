# ArenaOS — Vision

## What ArenaOS is

ArenaOS is a general-purpose operating system for x86-64 UEFI computers, written
from scratch in Rust. It is **not** a Linux distribution, **not** built on the
Linux kernel, and **not** a Unix clone. It is an independent system with its own
kernel, boot process, memory model, driver architecture, userspace, APIs, and
identity.

The long-term goal is a usable, desktop-capable operating system. The governing
engineering value is that every layer must be *earned*: each subsystem is built
on a tested, stable foundation below it, and the system is bootable and
verifiably working at every commit.

## Core beliefs

1. **Security is architectural, not a feature.** All access to kernel and
   service resources flows through capabilities — unforgeable, delegable
   handles with explicit rights. There is no ambient authority: a program can
   only touch what it was given.
2. **The kernel should be small enough to reason about.** The kernel owns
   memory, threads, scheduling, IPC, capabilities, and interrupt dispatch —
   nothing more. Drivers, filesystems, and network stacks are recoverable
   userspace services. A crashed audio driver must not be able to take down the
   machine.
3. **Correctness over cleverness.** Simple, understandable foundations beat
   clever fragile ones. Every milestone must be demonstrated by automated
   tests that would fail if the subsystem were faked.
4. **Modern APIs, not inherited ones.** We do not assume POSIX or Unix
   semantics are correct. Handles are typed objects with rights, spawning is
   explicit (no `fork` inheritance model), errors are typed, and I/O is
   designed asynchronous-first. A POSIX compatibility subsystem may exist
   someday — as a user, of the real API, never as its master.
5. **Performance is designed in, not bolted on.** A microkernel's historic
   weakness is IPC cost; ours is addressed from day one: synchronous
   rendezvous for control, badge-based notifications for events, and
   shared-memory rings for bulk data.
6. **The project must survive its authors.** Twenty-year maintainability means:
   ADRs for every significant decision, tests for every subsystem, small
   modules, no undocumented magic, and a bootable tree at all times.

## What makes ArenaOS distinct

These are commitments, not marketing:

- **Capability-based everything.** Kernel objects, files, sockets, and devices
  are all accessed through capabilities with rights that can be attenuated on
  delegation. Sandboxing is not a bolt-on; it is what happens when you hand a
  process fewer capabilities.
- **Recoverable system services.** Services are supervised and restartable.
  State that matters is designed to survive service restarts.
- **Explicit application permissions.** Applications declare what they need;
  users grant it; the system enforces it through capability attenuation at
  spawn time.
- **No Unix inheritance model.** Processes are spawned with exactly the
  handles they are given. There is no `fork`, no implicit fd leaking, no
  signals-as-API.
- **Typed, versioned kernel ABI.** A small fixed syscall surface for kernel
  objects; everything else (files, networking, graphics) is userspace protocol
  over IPC, free to evolve without touching the kernel ABI.

## Target

- **First hardware target:** x86-64 PCs booting via UEFI, developed and tested
  in QEMU.
- **Later:** ARM64, real hardware, graphics desktop, networking, package
  management — in that general order, gated on the stability of what
  underneath.

## Non-goals (permanent)

- Binary or source compatibility with Linux.
- POSIX conformance as a design requirement.
- Reusing another OS's kernel, init, drivers, or coreutils as our foundation.
- Shipping cleverness we cannot test.
