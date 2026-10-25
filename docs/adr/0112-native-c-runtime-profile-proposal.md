# ADR-0112: Native C runtime profile and C-facing stdio

Status: **Accepted by M (2026-10-25)** — the reconciled Phase 16 bounded
native C-runtime architecture, including its documented limitations. This
approval accepts the contract; it does not accept any implementation,
qualify Phase 16, or authorize merging. The runtime remains on branch
`arena/2b4ae1a5-arenaos`; it has **not** landed on `main`.

## Current decision (authoritative, 2026-10-25)

Where the historical body below conflicts with this section, this section
wins. The "Phase 16 reconciliation" section near the end (dated
2026-10-24) is part of the accepted decision text.

1. **One canonical runtime** (`experiments/c-runtime/`, provenance
   `docs/phase16/PROVENANCE.md`), two ring-3 CPU profiles: the integer
   profile (mandatory no-FP ISA audit) and the baseline FP profile
   (`-march=x86-64 -msse2 -mno-avx -mno-avx2 -mno-avx512f`; Clang via zig:
   freestanding default + AVX refusals), under ADR-0113 (x87/MMX/SSE/SSE2;
   AVX unsupported). The kernel stays integer-only.
2. **Bounded library subset**: exactly the 110 symbols of
   `docs/compat/C2-LIBC-COVERAGE.md`. Unsupported functions refuse
   (`ENOSYS`/`EINVAL`/`thrd_error`) or are absent (`_exit`, `%f`,
   `fopen`, recursive/timed mutexes, `tss_*`, `CLOCK_REALTIME`, `time`,
   `clock`); none returns fake success. No pthreads, no `setjmp`, no
   locale, no filesystem authority. Not full POSIX or complete C11.
3. **Threading**: bounded C11 `threads.h` over ADR-0106/0107; kernel bound
   of 4 user threads per process (measured, not raised).
   `cnd_timedwait` reads `CLOCK_MONOTONIC` (documented deviation).
4. **TLS remains runtime-managed** by `crt0` (`local-exec`); the loader
   still ignores `PT_TLS`. Not a Phase 16 blocker.
5. **Startup validation**: ARST v2 as implemented (exit 120/121/122).
   The Python oracle matches the native validators; no native validation
   was weakened; the wire format is unchanged.
6. **Streams**: capability-backed stdio; wire layout per accepted
   ADR-0111 (ASTR v2, 1024-byte rings). Applications built for the v1
   layout are refused at stream setup (exit 58), never silently run.
7. **No privileged serial fallback** in the positive demonstration;
   `SYS_DEBUG_WRITE` only via explicit legacy opt-in object. The
   syscall's own fate remains a separate open decision (review question 1).
8. **Termination**: `exit` runs `atexit` (≤32, reverse) and flushes;
   `_Exit`=`abort` skip handlers (given/134); `thrd_exit` ends only the
   caller; **process status = status of the last thread to exit**.

## Historical proposal text (2026-10-08, context only)

Milestone context: Phase 13 baseline (branch `arena/f89ea987-arenaos`). The
text below is the original proposal and is preserved as historical context;
statements superseded above (no FP state, 768-byte rings, "proposal only")
no longer describe the accepted contract.

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
4. Numbering: this proposal was first filed as ADR-0110. C2.1 (2026-10-11)
   renumbered it to **0112**, the next number free on `main`, on the Phase-14
   branch, on the Arena 1 graphics branch, and on this branch. ADR-0110 is
   already taken by `0110-static-native-pie-and-kernel-placement.md` (accepted
   Luna work). That accepted ADR is not changed by this renumbering. Status
   remains **Proposed**; the renumbering does not accept this proposal.
   Accepted ADRs are never renumbered.

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

## Phase 16 reconciliation (2026-10-24)

This addendum reconciles the proposal with current ArenaOS. It does **not**
accept the ADR; status remains **Proposed** pending M's explicit acceptance.
Phase 16 imported the runtime to `main` (provenance:
`docs/phase16/PROVENANCE.md`) and is qualifying it under the contract below.

1. **CPU profile.** ADR-0113 is **Accepted** (M, 2026-10-24, Phase 15,
   merged via PR #4): eager per-thread FXSAVE64 for x87/MMX/SSE/SSE2; AVX
   remains unsupported; CPL3 faults (`#MF`/`#XF`/`#UD`) are contained with
   status `0x100+vector`. The C1-era "no SSE/x87" build profile above is
   therefore split into two supported-by-evidence profiles under *one*
   runtime: (a) the **integer profile** (historical flags, mandatory no-FP
   ISA audit) for the existing runtime and apps, and (b) the **baseline FP
   profile** for applications that need it: `-march=x86-64 -msse2
   -mno-avx -mno-avx2 -mno-avx512f` (Clang via zig uses the freestanding
   target default + AVX refusals), with CPUID gating for anything above
   SSE2 and an instruction-aware positive audit (SSE2/x87 required, AVX
   forbidden). `docs/compat/C1-FPSIMD-BOUNDARY.md` "no FP state" statements
   are historical and superseded by ADR-0113. The *kernel* stays
   integer-only (ADR-0012/0113); this profile is ring-3 only. Phase 15
   qualification scope (pinned locally modified QEMU 11.1.2 TCG) does not
   by itself qualify C applications or upstream/hardware backends.
2. **Bounded library subset.** The proposed supported set is exactly the
   110 symbols of `docs/compat/C2-LIBC-COVERAGE.md` (defined in both
   compilers' archives), with per-function evidence levels recorded there.
   Unsupported functions refuse (`ENOSYS`/`EINVAL`/`thrd_error`, or are
   absent, e.g. `_exit`, `%f` formatting, `fopen`, recursive/timed mutexes,
   `tss_*` keys, `CLOCK_REALTIME`, `time`/`clock`); none returns a fake
   success. No pthreads, no `setjmp`, no locale, no filesystem authority.
   This ADR does not propose full POSIX or complete C11.
3. **Threading subset.** C11 `threads.h` as bounded in the coverage doc,
   over `SYS_THREAD_*` and sync domains (ADR-0106/0107), 4 user threads
   per process (kernel bound, measured, not raised). `cnd_timedwait`
   deadlines are read on `CLOCK_MONOTONIC` (documented deviation from
   `TIME_UTC`; no wall-clock grant exists).
4. **TLS remains runtime-managed** (decision 4 above, unchanged):
   `crt0` installs per-thread TLS (`local-exec`, FS-base via ADR-0085
   bindings); the loader still ignores `PT_TLS`. Review question 3 stays
   open and is *not* a Phase 16 blocker because the runtime path is
   guest-tested (C1 T8).
5. **Startup validation.** ARST v2 gate as implemented (exit codes
   120/121/122 for stack/TLS/record failures; live-cap inventory checked
   against the record; flag↔descriptor agreement for streams/sync). The
   Phase 16 M9 fix aligned the Python oracle (`tools/startup_abi.py`) to
   the native validators (7 descriptors, roles 6/7/8, kind 16, flags mask
   31); no native validation was weakened and the wire format is unchanged.
   Graphical service identification remains B-GFX-1 (no slot guessing).
6. **Streams.** Capability-backed stdio over ADR-0104; wire layout per
   ADR-0111 **Accepted 2026-10-25** (ASTR v2, capacity 1024; supersedes
   this ADR's historical "768-byte ring" downside). Live-stdin guest
   evidence is a Phase 16 requirement, in progress.
7. **No privileged serial fallback in the positive demonstration.**
   `SYS_DEBUG_WRITE` is reachable only via the explicit legacy opt-in
   object; Phase 16 removed the last weak reference from ordinary images
   (hook-pointer dispatch; `nm -u` empty under GCC and Clang). Review
   question 1 (the syscall's own fate) remains a separate open decision
   for M; the C path does not depend on its answer.
8. **Termination semantics (exact).** `exit` runs `atexit` handlers (≤32,
   reverse order) and flushes; `_Exit`/`abort` skip handlers (statuses
   given/134). `thrd_exit` ends only the calling worker. **Process exit
   status is the status of the last thread to exit** (kernel rule; there
   is no separate whole-process termination syscall — adding one would
   need its own ADR). Documented consequence: a detached worker that
   outlives `main`'s return determines the process status; C2.1.2's
   refcounted detach + thread-count observation is the supported pattern.
   Detached-thread memory is freed exactly once by the last reference
   (bounded stress-verified; not a proof for all schedules).

Decision: **Accepted by M on 2026-10-25** (architecture approval; see the
status block). ADR-0111 was approved the same day; neither acceptance
implies the other, and neither accepts an implementation.
