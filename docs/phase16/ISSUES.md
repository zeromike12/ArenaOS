# Phase 16 known-issue register

Each assigned issue: current reproduction on Phase 15 `main` + imported
runtime, classification, disposition. Labels: **FIXED** (regression test
named), **PINNED-LIMIT** (defect pinned by test, documented limitation, fix
needs M/C decision), **VERIFIED-OK**, **OPEN** (needs guest access or a
decision), **HANDOFF** (Phase 17).

## I-1 — Standard-stream ring counter wrap → **FIXED (development-verified)**

Originally reproduced: both implementations indexed the 768-byte ring
with `counter % 768` over `u32` counters; because 2³² mod 768 = 256,
unread bytes could be overwritten at the wrap. M accepted ADR-0111 on
2026-10-25, and ASTR v2 is now implemented coherently in Rust and C:
version 2, three 1,024-byte power-of-two rings in one page, wrapping
`u32` occupancy, `counter & 1023` indexing, and explicit refusal of
v1/unknown versions or wrong capacity. Regression coverage includes
Rust and C host tests for seeded near-wrap transfers with unread bytes,
empty/full/partial/backpressure/EOF/closed-reader behavior, malformed
occupancy/layout refusal, cross-language layout checks, and an explicit
v1-indexing negative control; the C1 guest app also covers the near-wrap
path. Fresh development evidence (not qualification; B1 remains open)
is in `receipts/dev-guest-2026-10-25-reconstructed/`.

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
the current reproduced HOST-ONLY 1707-check suite. Imported unchanged.
Remaining gap is Arena 2's own: no *guest* two-candidate ambiguity run
exists. A guest negative (two RW notifications → `ARENA_E_NO_WAITER`, no
misbinding) is still required for qualification; development guest
execution is unblocked, but producing that launcher record was not part
of the reconstructed targeted suites run on 2026-10-25.

## I-4 — Clang weak undefined `arena_legacy_write` → **FIXED**

Reproduced exactly as root-caused by Arena 2 (gcc ELFs: no undefined;
clang ELFs: `w arena_legacy_write`). Fixed with Arena 2's proposed design:
hook pointer owned by `rt.c`, set only by `arena_legacy_serial_enable()`
in `legacy_debug.c`; `rt.o` no longer references the symbol at all.
Verification: `nm -u` empty for **both** compilers' ordinary apps after
rebuild; host suite and ABI check re-run green; guest harness's
legacy-symbol refusal in `guest_c2.py` now trips on nothing. The debug
syscall remains reachable only through the explicit legacy opt-in.

## I-5 — Live stdin behavior → **FIXED (development-verified)**

Implemented as a narrow Desktop path over the existing capability-owned
native streams: a windowed stream app receives a pre-created console
window, keyboard bytes go only to the focused window's owning
AppInstance, and Ctrl-D closes that instance's stdin writer only after
already-queued bytes drain. `experiments/c-runtime/guest_stdin.py`
drives real QMP keyboard input to two simultaneously running signed C
apps and requires exact attribution/order, no delivery to the unfocused
instance, burst/backpressure handling, EOF line/byte counts, no-grant
refusal, and explicit ASTR-v1 refusal. GCC and Clang both passed with
exits `[58, 73, 73, 58]` in the 2026-10-25 reconstructed development
receipts. This is not a general shell or POSIX terminal layer, and B1
still prevents calling it qualification evidence.

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

## I-8 — Pre-created console window lost its window wake → **FIXED**

Found by the full Phase 13 regression on the first reconstructed
development image. Pre-creating a stream app's primary window queued a
Focus event and let Desktop spend its one edge-triggered `WOKEN`
notification before the app adopted the window; the app could consume
that badge during unrelated startup helper/stream waits and then block
forever with window events still pending. Desktop now suppresses that
window wake at pre-creation and re-arms it when the app adopts the
window through its own `Frame::Create`; stdin uses its separate stream
badge and is unaffected. The full Phase 13 suite passes on the corrected
development ESP, including both targeted extra-window closes.

## I-9 — Test-judge false positives from unrelated serial/input lag → **FIXED**

Two harness defects were isolated without weakening semantic checks:
(1) the FP judge rejected any serial containing `SKIP`, including the
unrelated boot service line `configup: SKIP`; it now rejects only
`[c-fp] … SKIP`, with self-test controls on both sides (12 cases); and
(2) Phase 13 could click an All Applications search result while lagging
keyboard-device events were still queued behind the separate pointer
device, after which those keys correctly became the newly focused app's
stdin. The harness now waits for the non-empty search field to stop
changing before clicking. Neither change alters a guest assertion or an
expected exit sequence.

## Environment blockers

**B2 is resolved** for development execution by the exact pinned QEMU
under an isolated glibc (`BACKEND-RECOVERY.md`); the host system glibc
was not replaced. **B1 remains open**: official Rust 1.97.0 target
components are unavailable here, so canonical rebuild/qualification,
protected-identity reproduction on a canonical build, the full
registered regression qualification run, 100/100 final-artifact
stability, fresh-archive qualification, and any acceptance/merge remain
blocked. Development-image results must keep their explicit
non-qualifying label.
