# Phase 16 guest integration runs — DEVELOPMENT evidence (2026-10-24)

> These runs used a development kernel (`tools/build_phase16_dev.sh`:
> committed protected Phase 14 fixture verified, not rebuilt) because B1
> blocks the official build in this sandbox (RECOVERY-LOG.md). Every
> guest-side assertion is the unchanged Arena 2 harness judge; the only
> substitution is `mtest.build` → prebuilt ESP
> (`guest_phase16_dev.py`). All results below must be re-run on an official
> build before they count as qualification evidence.

## Identities

- Backend: pinned patched QEMU `e0ea2b1f…` under isolated glibc 2.40
  (BACKEND-RECOVERY.md); package firmware.
- Development ESP `7d6fb823…`, EFI `f29d7d8a…` (kernel ISA audit PASS in
  `receipts/dev-guest/dev-build.log`).

## Executed in the guest (signed APB1, installed + launched from desktop)

| Suite | Compiler | Verdict | Details |
|---|---|---|---|
| C1.2 native (`guest_native`) | GCC | **1/1 exact PASS** | 93 checks, 9/9 groups; exits [58, 57] |
| C2 multi-app (`guest_c2`) | GCC | **1/1 exact PASS** | console 88 checks 9/9, xxHash digest exact; exits [58, 58, 61, 63] |
| C2 multi-app | Clang 22 | **1/1 exact PASS** | same contract |
| FP integration (`guest_fp`) | GCC | **1/1 exact PASS** | 35 checks, 0 failed, 0 skips; exits [58, 71, 72] |
| FP integration | Clang 22 | **1/1 exact PASS** (2nd attempt) | first attempt failed at desktop launch step ("All Applications did not launch fp-nostream") before any app ran; recorded in `fp-dev-clang.log`, rerun passed — transient harness/UI flakiness, kept visible |

The FP positive app exercised: FP arithmetic/x87 with real generated code,
thread create/join (main+3) with thread-owned FP state, cooperative-yield
and condition-variable blocking transitions with architectural XMM0–7 +
x87 state verified by FXSAVE after rescheduling. Timer-preemption state
preservation is **not** independently claimed (see app comments). The FP
mutant (deliberately corrupted probe value) was detected in-guest:
`VERDICT FAIL`, failed=18, exit 72 — the guest negative control works.
(Note: app verdict wording is `VERDICT`, not `RESULT`, because `mtest`
treats any `RESULT FAIL` substring as a fatal milestone marker.)

## Not yet executed / remaining gaps

- **Live stdin through a granted stream**: the desktop currently offers no
  input path into a headless stream app's stdin channel; C2 G2 classifies
  EOF/empty only. A real interactive demonstration needs a desktop-side
  stdin feeder (or a terminal-hosted stream app) — proposed, not faked.
- Stream ASTR v2 (ADR-0111): designed + host-modeled only; no wire change
  pending M's decision. Guest stream tests near counter wrap are specified
  in the ADR.
- Host check totals (1099 + 1697 HOST-ONLY) are unchanged; the incorrect FP
  test has been replaced (host 18/18 new comparator/probe checks).
- Full regression counts, final-artifact verification, archive
  re-extraction: pending an official build (B1).
