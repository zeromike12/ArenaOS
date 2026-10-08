# ArenaOS C compatibility: audit, toolchain, prototype, strategy, roadmap

Branch: `arena/f89ea987-arenaos` (from `arena/phase13-native-app-maturity` @ 74ace4f).
Status: analysis and experimental prototype. **No production kernel, userspace,
or tooling file is modified.** The experiment lives in
[`experiments/c-runtime/`](../../experiments/c-runtime/README.md). The
main branch is not touched and nothing is merged or released.

## C1 (native application platform): current status

Read these first for the C1 milestone on this branch:

| Document | Purpose |
|---|---|
| [C1-PLAN.md](C1-PLAN.md) | Scope, sub-milestones C1.1–C1.5, verification rules, escalations, remaining gaps |
| [C1-INTEGRATION-CONTRACT.md](C1-INTEGRATION-CONTRACT.md) | What ArenaOS already provides to a native C app, labelled EXISTING, EXPERIMENTAL-BINDING, or MISSING |
| [C1-FINAL-REPORT.md](C1-FINAL-REPORT.md) | Implemented, tested, proposed, and blocked, with hashes and exact guest counts |
| [C1-SDL3-HANDOFF.md](C1-SDL3-HANDOFF.md) | For Arena 1: what SDL3 needs and what is missing (no port) |
| [C1-FPSIMD-BOUNDARY.md](C1-FPSIMD-BOUNDARY.md) | CPU FP/SIMD boundary audit; core-OS dependency for Luna after Phase 14 |
| [ADR-0111](../adr/0111-stream-ring-counter-wrap.md) | Proposed: stream ring u32 counter wrap (blocker, wire-format change) |

Current C1 evidence (see the final report for hashes and the exact commands):
native app GUEST 5/5 clang and 5/5 gcc; crt-probe GUEST 3/3 per compiler; host
HOST-ONLY 598 checks; static audits rc 0 with 0 SIMD or x87 instructions in every
C image and in the kernel EFI.

The rest of this page is the earlier prototype milestone (Phase 13 era). Its
numbers are a snapshot and are superseded by the C1 results above, where they
differ (for example, the host count is now 598, not 42).

## Documents

| # | Document | Deliverable |
|---|---|---|
| 01 | [Architecture audit](01-architecture-audit.md) | C runtime audit: startup, syscall ABI, capabilities, memory, threads, TLS, stdio, filesystem, lifecycle, time; findings F1–F10 |
| 02 | [Toolchain feasibility](02-toolchain-feasibility.md) | GCC and clang/LLD targeting ArenaOS: target spec, link line, startup objects, runtime deps, static audit results, bugs found |
| 03 | [Prototype and native C milestone](03-prototype-report.md) | Experimental runtime, host results, guest-executed probe output, non-interference evidence, limitations |
| 04 | [Compatibility strategy](04-compat-strategy.md) | Options (a) native ports, (b) brokered POSIX subset, (c) Linux ELF compat; SDL3, SuperTuxKart, Blender dependency analysis |
| 05 | [Staged roadmap](05-roadmap.md) | Stages 0–5: milestones, existing vs missing, security risks, core-OS and graphics dependencies, phase boundaries |

Proposal: [ADR-0110 — Native C runtime profile and C-facing stdio](../adr/0110-native-c-runtime-profile-proposal.md)
(**Proposed**, not accepted; for architecture review).

## Evidence labels

- **GUEST**: executed under ArenaOS in QEMU. Serial logs are in
  `experiments/c-runtime/build/` (git-ignored).
- **HOST-ONLY**: host test binary (glibc differential, ASan+UBSan). Not an
  ArenaOS execution.
- **STATIC**: checked from ELF or disassembly without running.
- **PRIMARY / SECONDARY**: source quality for external claims (report 04).
- **PROPOSAL**: recommended, not landed.

## Results at a glance

| Question | Answer | Label |
|---|---|---|
| Does a freestanding static C program run on ArenaOS? | Yes. It printed, allocated and modified memory, and reported `CRT-PROBE RESULT PASS (6/6)`. | GUEST (GCC and clang/LLD) |
| Did the boot change? | No. Same suites (M1–M7, M11, M12) PASS, same services and FAIL-line count as the unpatched control. | GUEST |
| Is the exit status (37) observed? | No. The kernel does not print exit codes to serial. | not observed (F8) |
| Host allocator and runtime tests (prior snapshot; current C1 count is 598) | 42 checks PASS under ASan+UBSan at the time. | HOST-ONLY |
| No SSE/x87 in the guest ELFs? | Yes, 0 instructions in either build, after `-mno-sse` family. | STATIC |
| Are ABI constants consistent with `abi.rs`? | 13 constants, 0 mismatches (accepts constants absent from the kernel, see F9). | STATIC |
| Is `SYS_DEBUG_WRITE` a safe C stdio? | No. It has no capability check (F1). | CODE |
| Are SDL3, STK and Blender portable now? | No. Blockers are listed in report 04 and 05. | analysis |

## Bugs the guest run exposed (fixed)

1. TLS memsz in the linker script omitted alignment padding, so the thread
   pointer was 16 bytes off. Found by GUEST, proven by a STATIC negative control.
2. zig clang emitted SSE despite `-mgeneral-regs-only`. Fixed with the
   `-mno-sse` family; the audit enforces it.
3. A probe check measured against zero, but the runtime's own TLS block is live.
   Fixed with deltas.

Full table: report 03 §5 and report 02 §6.

## Not done (explicit)

- No Linux binary compatibility, no dynamic linker, no PIE/ASLR (Luna, Phase 14).
- No SDL3, SuperTuxKart or Blender port. No graphics work (Arena 1).
- No ABI change landed. Proposals are in ADR-0110 only.
- No merge, no release, no change to `main` or Luna's branch.
- No `run.py all` in a single invocation was run; `build`, `audit`, `abi`,
  `host`, and each compiler's `guest` were run separately.
- The clang guest run was executed once, and the gcc run once; there is no
  stability study.
