# Phase 16 known-issue register

Each assigned issue: current reproduction on Phase 15 `main` + imported
runtime, classification, disposition. Labels: **FIXED** (regression test
named), **PINNED-LIMIT** (defect pinned by test, documented limitation, fix
needs M/C decision), **VERIFIED-OK**, **OPEN** (needs guest access or a
decision), **HANDOFF** (Phase 17).

## I-1 — Standard-stream ring counter wrap → **PINNED-LIMIT**

Reproduced. Both implementations index the 768-byte ring with
`counter % 768` over `u32` counters (`userspace/arena-runtime/src/streams.rs:18`,
C `src/streams.c:138,178`). 2³² mod 768 = 256, so at the wrap unread bytes
can be overwritten (host pin: `KNOWN-DEFECT ring u32-wrap discontinuity
reproduced`, still emitted by `run.py host` at this baseline). Fix is a
wire-format change (proposed ADR-0111, options: u64 counters v2 /
power-of-two capacity / quiescent reset) — **decision requested from M/C**;
not landed, not silently adopted. Until decided, the bounded Phase 16
stream contract must state the limit plainly: a single channel is not
safe past 2³² bytes in one stream lifetime; streams must not be
advertised as unbounded-safe. This defect is the reason Phase 16 cannot
ask for stream acceptance beyond that bound.

## I-2 — Startup descriptor-count mismatch (M9) → **FIXED**

Reproduced and wider than reported. Native validators
(`arena-platform startup.rs` `CAPABILITY_MAX=7`, C `ARENA_ARST_CAPABILITY_MAX=7`)
accept up to 7 descriptors, kinds 1–13 plus 16 (SyncDomain), roles 1–8,
flags mask 31. The independent Python oracle (`tools/startup_abi.py`) still
had the pre-stream/sync contract: `CAP_MAX=4`, kinds 1–13 only, roles 1–5,
flags mask 7 — it could not encode *any* real stream+sync C application
record. The Python side was the stale one. Fixed by aligning the oracle to
the native validators (no wire or validation change on the native side);
regression tests: `test_m9_seven_descriptor_stream_sync_record_roundtrips`,
`test_m9_over_bound_and_unknown_records_still_refuse` in
`tools/test_startup_abi.py`.

## I-3 — Notification selection / ambiguous candidates → **VERIFIED-OK (runtime), OPEN (guest ambiguity)**

The runtime selects the sole role-Other/RW Notification and refuses zero
or ambiguous candidates (`arena_startup_pick_notification`,
`src/startup_query.c`); host tests (`test_notification_selection`) pass in
the reproduced 1697-check suite. Imported unchanged. Remaining gap is
Arena 2's own: no *guest* two-candidate ambiguity run exists. A guest
negative (two RW notifications → `ARENA_E_NO_WAITER`, no misbinding) is
queued for P16.5; it needs a launcher-produced record, so it rides on the
kernel-build unblock.

## I-4 — Clang weak undefined `arena_legacy_write` → **FIXED**

Reproduced exactly as root-caused by Arena 2 (gcc ELFs: no undefined;
clang ELFs: `w arena_legacy_write`). Fixed with Arena 2's proposed design:
hook pointer owned by `rt.c`, set only by `arena_legacy_serial_enable()`
in `legacy_debug.c`; `rt.o` no longer references the symbol at all.
Verification: `nm -u` empty for **both** compilers' ordinary apps after
rebuild; host suite and ABI check re-run green; guest harness's
legacy-symbol refusal in `guest_c2.py` now trips on nothing. The debug
syscall remains reachable only through the explicit legacy opt-in.

## I-5 — Live stdin behavior → **OPEN (documented limitation)**

Unchanged from Arena 2: pushback/`fgets` error paths are host-tested, but
no controlled guest input fixture has ever driven live stdin to a C app
through the granted stream (an idle read blocks; G2 only classifies).
Phase 16 plan: drive stdin from the desktop terminal path in the P16.5
guest demo (typed line → C app echoes a checksum line), asserted on serial
+ screenshot. Until that runs, coverage docs keep saying "live stdin:
untested". Not a fake-pass candidate.

## I-6 — Process exit vs worker-thread exit → **VERIFIED-OK (with one documented kernel rule)**

`thrd_exit` ends only the worker (`SYS_THREAD_EXIT` per-thread; guest G6
status 7 with the app continuing — Arena 2 evidence, code imported
unchanged). Process exit status is the **last thread to exit** (kernel
rule), so a detached worker outliving `main`'s return can determine the
process status; C2.1.2's refcounted records + explicit thread-count
observation are the supported pattern, documented in
`docs/compat/C2-LIBC-COVERAGE.md`. No kernel termination syscall is added
(would need an ADR). Phase 16 adds one guest assertion at P16.5: `exit(63)`
from `main` with a detached worker still parked must be reported honestly
(process ends when the last thread ends), not papered over.

## I-7 — Graphical surface/compositor identification → **HANDOFF (Phase 17)**

Unchanged blocker B-GFX-1: graphical descriptors all carry role Other, so
"surface" vs "compositor" is indistinguishable in ARST v2 and must not be
guessed by slot. Arena 2's comparison stands (`C2.1-B-GFX-1-HANDOFF.md`):
protocol-level identification over the BadgedEndpoint (recommended, no
ABI change) vs signed-manifest layout vs future ARST extension (ADR).
Phase 16 does not decide this; `ARENA1-HANDOFF.md` (P16.6) carries it to
the SDL3 milestone with the runtime facts Arena 1 needs. Count-by-kind
(`arena_startup_count_kind`) remains available and host-verified.

## Environment blockers

B1/B2 in `BASELINE.md` §4 gate all guest execution (P16.5) and final
qualification (P16.6). They are environment/allowlist issues, not code
defects; both have concrete resolution paths for M.
