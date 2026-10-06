# ADR-0085 — Native per-thread TLS through FS.base

**Status: Accepted for Phase 12.5 (single-user-thread guest handoff proof passes; user-thread creation and TLS uniqueness remain open).**

**Date:** 2026-10-05. **Milestone:** Phase 12, native runtime.

**Related decisions:** ADR-0014, ADR-0017, ADR-0080, ADR-0083, ADR-0084.

## Problem

The native process/runtime contract has no TLS block, per-thread base, or
scheduler ownership rule. x86-64 `swapgs` is already a security-critical
syscall-stack mechanism: user GS.base is zero and the kernel uses the paired
GS MSRs for its per-CPU syscall scratch. TLS must not repurpose GS or let one
process observe another thread's base. The kernel's syscall user-range checks
are per thread, so accepting a pointer solely because it is numerically
canonical would be unsafe.

## Decision

- Native user TLS uses the architectural `IA32_FS_BASE` MSR. GS and
  `swapgs` remain unchanged. Kernel threads start with FS.base zero; every
  scheduler thread owns a saved `fs_base` field. On a context switch the
  outgoing live MSR is recorded when its thread remains live and the incoming
  thread's value is installed before its context resumes. A terminating
  thread's value is discarded with that thread.
- Add syscall 53, `SYS_TLS_SET(base)`, with all five reserved argument words
  required to be zero. `base == 0` clears TLS. Otherwise the kernel accepts
  only a 16-byte-aligned address whose header lies in the current thread's
  registered user regions and whose live PTE is PRESENT|USER|WRITE|NX, not
  PCD/PWT device memory. Missing process, bad range, read-only/code/MMIO page,
  or noncanonical address returns `STATUS_BAD_ARG` without changing the
  current FS.base. It grants no mapping, capability, or cross-process access.
- `arena-runtime::tls::ThreadControlBlock` is 16 bytes and 16-byte aligned:
  `FS:0` is a self pointer and the first application word is at `FS:8`.
  Applications may extend this layout only under a separate runtime ABI
  version. `install` writes the self pointer and asks the kernel to validate
  the mapping. `current()` reads FS:0 and is only valid after a successful
  install; the kernel does not expose a TLS getter.
- This is a minimal ArenaOS native contract, not ELF `PT_TLS`, GNU TLS,
  `thread_local!` compiler lowering, dynamic linking, libc, POSIX, or Linux
  compatibility. No user-thread create syscall is added by this ADR.
- Because user-memory regions are still registered per kernel thread, the
  proof qualifies only the current single-user-thread process path. Any future
  user-thread facility must establish exact per-thread TCB allocation and
  lifetime, user-map validation in every thread, process-exit cleanup, and
  independent two-user-thread tests before making TLS/heap pointers shared.

## Verification

- Host runtime tests freeze TCB size, alignment and field offsets. Target
  Clippy with warnings denied and the no_std proof image build pass.
- The independent M12 ring-3 guest rejects a 16-byte-aligned read-only ELF
  pointer and a kernel-half pointer, proving failed SYS_TLS_SET calls leave an
  already-installed TCB intact. It then arms/waits on an explicit Notification
  timer, is switched to the kernel M12 waiter, resumes, and verifies both
  `FS:0` and its thread-local sentinel. It clears TLS before exit.
- The kernel checks the fired timer is retired, child capability occupancy is
  exact, and all process/map/frame/SharedRegion state returns to baseline.
  `tools/test_m12_startup.py` passes M12 6/6, plus m1–m7 and m11 regression
  markers in that exact boot.

## Consequences

- FS.base is now explicit per-thread state rather than an undocumented CPU
  residue. The old syscall GS pair retains its original transition contract.
- FS.base is a pointer into already-mapped caller memory, not a capability or
  authority to another process. PTE permission and thread-region checks occur
  before WRMSR; failed requests do not mutate the old value.
- This closes the basic TLS base/lifecycle gate for the current one-thread
  process model only. Process-wide region accounting, a user-thread runtime,
  TLS uniqueness under concurrent user threads, complete VM semantics and the
  final Phase-12 resource budget remain separate open gates.
