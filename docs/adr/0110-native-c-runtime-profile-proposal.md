# ADR-0110: Native C runtime profile and C-facing stdio (proposal)

Status: Proposed
Date: 2026-10-08
Milestone context: Phase 13 baseline (branch `arena/f89ea987-arenaos`). This is a
proposal only. It does not change the production kernel, the syscall ABI, or
the ELF loader. Experimental evidence is in `experiments/c-runtime/` and
`docs/compat/`.

## Problem

ArenaOS has no supported way to write a native program in C. An experimental
freestanding C runtime now runs on ArenaOS under QEMU (`CRT-PROBE RESULT PASS
(6/6)` with both GCC and clang/LLD; see `docs/compat/03-prototype-report.md`).
That prototype works only because it has two shortcuts:

1. Its stdout goes through `SYS_DEBUG_WRITE`, a raw serial backdoor with no
   capability check (ADR-0017 calls it temporary). Any process can use it.
2. It installs TLS itself, because the ELF loader ignores `PT_TLS`.

Before anyone builds a C library, a C stdio, or a port on top of this, the
project needs to decide what the C contract is. Left implicit, the shortcuts
become the contract.

## Options considered

**A. Keep the prototype experimental.** No decision. Lowest risk, but the
`SYS_DEBUG_WRITE` shortcut and the loader's TLS gap remain open-ended, and
every later C effort re-decides them.

**B. Propose a native C profile, with C stdio on native byte streams (ADR-0104)
and an explicit stream capability, and retire the debug backdoor for C
programs.** Build the C profile on the existing mechanisms: static ET_EXEC
(ADR-0108), SharedRegion rings with notifications, and explicit capability
grants at spawn. No new kernel object type is needed.

**C. Add a hosted libc or POSIX personality in the kernel.** Rejected. It
would put path and descriptor authority into the kernel, which contradicts the
capability model. It also duplicates work that belongs in brokered userspace
services (report 04, option b).

**D. Add a dynamic linker for C.** Rejected for this milestone. ADR-0108 and
Luna's Phase-14 work own loader and PIE scope; this proposal must not duplicate
it.

## Decision (proposed)

1. **Adopt, as an experimental profile, the freestanding static C contract**
   recorded in `docs/compat/02-toolchain-feasibility.md`: static `ET_EXEC`
   at 0x200000, `-ffreestanding -nostdlib -fno-pic -fno-pie`, SSE and x87
   disabled, `-ftls-model=local-exec`, and the link line. This needs no kernel
   change.
2. **Propose a C-facing stream binding** over the Phase-13 native byte-stream
   SharedRegion (ADR-0104). A C program would receive stdin, stdout and stderr
   as explicit stream capabilities through the existing Startup ABI v2
   descriptor slots. This is a proposal for a runtime binding only. It adds no
   syscall.
3. **Propose that `SYS_DEBUG_WRITE` must not be part of any C-facing
   contract.** A follow-up ADR must decide whether it is capability-gated or
   removed from non-debug builds. The current implementation has no check at
   all (report 01, finding F1). That is a security finding, and it is
   recorded here so it is not forgotten.
4. **Keep TLS runtime-managed for now.** The prototype's `crt0` installs TLS.
   Loader support for `PT_TLS` would be an additive loader change, so it is
   left as an open option for a separate ADR with Luna's loader owners
   (report 01, F3).
5. **Propose no new syscalls, no new kernel object types, and no change to
   the ABI v1 call-number registry in this ADR.**

## Reasoning

- It makes the C contract explicit before code depends on the shortcuts.
- Option B reuses the authority model. A C program gets exactly the streams it
  is granted and nothing else. Paths, PIDs and names still confer no authority.
- Measured evidence supports the feasibility of the profile on both compilers
  (guest-executed, report 03). It does not prove stream-based stdio, because no
  such binding exists yet.
- Keeping the syscall surface unchanged keeps this proposal reviewable on its
  own.

## Downsides accepted

- The C stdio binding has to be built and guest-qualified before the probe can
  stop using the debug backdoor. Until then, C output goes through the backdoor.
- A stream binding shares the 768-byte ring limit of ADR-0104. Large C writes
  are chunked.
- Keeping TLS in the runtime means each C runtime owns a fragile linker-script
  contract (the memsz bug in report 02, T2). The audit now guards it.

## Future implications

- Any future loader PT_TLS support, PIE support, or C thread API must come with
  its own ADR. Luna's Phase-14 PIE work is a prerequisite for relocated C
  images.
- The stdio decision constrains the SDL3 and other porting paths (report 04).
  Those paths need a stream or event binding that does not rely on the serial
  backdoor.
- Revisit this ADR if the debug backdoor is removed, if the loader gains PT_TLS,
  or if a C thread API is proposed.

## Review questions for the architecture reviewer

1. Should `SYS_DEBUG_WRITE` be capability-gated, or removed from non-debug
   builds? (Recommended: removed from the C path now; decided separately.)
2. Is the Startup ABI v2 descriptor slot the right way to hand C stream
   capabilities over, or should a C-specific descriptor be proposed?
3. Should the runtime or the loader own `PT_TLS` long term?
4. Numbering: this proposal takes the next free number on this branch (0110).
   If another branch lands 0110 first, renumber at integration. Accepted ADRs
   are never renumbered.

## C1 update (2026-10-08): status of each decision

This addendum records what the C1 work implemented and tested. It does **not**
accept the ADR. The decision list above is unchanged.

1. **Freestanding static C profile**: implemented as an experiment in
   `experiments/c-runtime/` (gcc and clang, static `ET_EXEC` at 0x200000,
   no SSE/x87). Static audit rc 0 for both compilers on the final tree
   (`simd_or_x87_count` 0 for every ELF). Guest-executed on the final sources
   (commit `bcdfb95`): the native application ran 5 of 5 exact PASS per compiler
   (exit lines `[58, 57]`); the crt-probe ran 3 of 3 exact PASS per compiler. See
   `docs/compat/C1-FINAL-REPORT.md` for the hashes and the earlier runs.
2. **C stream binding over ADR-0104**: implemented experimentally on the C side
   (`src/streams.c`, `src/stdio.c`). It uses the existing StreamSet and
   StreamWake grants and existing syscalls only. Guest-tested with the signed
   native application: granted stdout and stderr writes, stdin bounded reads,
   EOF, and backpressure. Still a proposal for landing.
3. **`SYS_DEBUG_WRITE` excluded from the C path**: the C1 native application
   never calls `arena_legacy_serial_enable()`, so no C1 application output
   reaches the debug syscall. The legacy boot probe still opts in. The kernel
   still has no capability check on `SYS_DEBUG_WRITE` (F1). The follow-up ADR
   that decides its fate is still **open**.
4. **TLS runtime-managed**: unchanged. The loader still ignores `PT_TLS` (F3).
   The C runtime installs TLS. Guest-tested per thread (group T8: four
   threads keep their own TLS value).
5. **No new syscalls, kernel objects, or ABI v1 changes**: held. C1 threads use
   `SYS_THREAD_CREATE/JOIN/DETACH/EXIT/YIELD/COUNT`; synchronization uses the
   existing sync-domain calls; memory uses `SYS_VM_*`. No kernel file was
   changed by C1.

New items raised by C1 (each has its own record):

- **Thread API (C1.3)**: the C thread and synchronization binding exists
  experimentally and is guest-tested (group T9: 34 checks, including quota
  refusal, condvar notify-all and notify-one, timed wait, detach, stale join,
  and a stale-handle rejection). Landing a C thread API still needs its own
  ADR, as the Future implications section already says.
- **Stream ring counter wrap**: a defect in the ADR-0104 protocol at 2^32 bytes
  per channel lifetime. Proposed fix options are in `docs/adr/0111-stream-ring-counter-wrap.md`.
- **FP/SIMD**: C code cannot use FP or SIMD. The kernel has no FP state save
  or restore, and this is a core-OS dependency for Luna's review after Phase 14.
  See `docs/compat/C1-FPSIMD-BOUNDARY.md`.
- **Heap size**: the C allocator reserves 16 MiB per process (C1.4). The
  prototype's 4 MiB heap is superseded. Legacy probe expectations were updated
  to match (see the final report).

Review question 1 above remains open. Question 3 (`PT_TLS` ownership) remains open.
