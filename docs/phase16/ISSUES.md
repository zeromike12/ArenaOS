# Phase 16 known-issue register

Each assigned issue: current reproduction on Phase 15 `main` + imported
runtime, classification, disposition. Labels: **FIXED** (regression test
named), **PINNED-LIMIT** (defect pinned by test, documented limitation, fix
needs M/C decision), **VERIFIED-OK**, **OPEN** (needs guest access or a
decision), **HANDOFF** (Phase 17).

## I-1 — Standard-stream ring counter wrap → **FIXED (canonical-verified)**

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
path. After B1 was resolved, the canonical GCC/Clang C1, C2, and
live-stdin guest suites passed at clean `d314d20…`, including explicit
ASTR-v1 refusal (see `receipts/canonical-2026-10-26/`). After the later
reserved-byte correction (I-10), the same GCC/Clang suites passed again
at clean `5aacbc8…`; the corrected-candidate receipts are in
`receipts/canonical-corrected-5aacbc8-2026-10-26/`. The earlier fresh
development evidence remains separately classified in
`receipts/dev-guest-2026-10-25-reconstructed/`.

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

## I-3 — Notification selection / ambiguous candidates → **FIXED/VERIFIED (host + guest)**

The runtime selects the sole role-Other/RW Notification and refuses zero
or ambiguous candidates (`arena_startup_pick_notification`,
`src/startup_query.c`); host tests (`test_notification_selection`) pass in
the corrected HOST-ONLY 1721-check suite. The previously missing guest
negative is now implemented in
`experiments/c-runtime/guest_notification.py` with
`app/c_notification_app.c`. A launcher audit found no production
windowed/headless/helper, APB1-capability, stream-grant, or
service-manager path that can emit two candidates, so the suite uses
the repository's restored-mutation pattern: production sole-candidate
boot PASS (exit 75), then a labelled Desktop fixture adds a distinct
second role-Other/RW Notification and the probe observes selection
`NONE`, empty-stdin peek `-1005`, immediate blocking
`ARENA_E_NO_WAITER=-1012`, and exit 76. GCC and Clang passed
canonically at `d314d20…` and again on the corrected candidate at
`5aacbc8…`; Desktop source and production EFI/ESP/profile restoration
is asserted byte-exact (see
`receipts/canonical-corrected-5aacbc8-2026-10-26/guest/notification/`;
the historical `d314d20…` receipts remain in
`receipts/canonical-2026-10-26/guest/notification/`).

## I-4 — Clang weak undefined `arena_legacy_write` → **FIXED**

Reproduced exactly as root-caused by Arena 2 (gcc ELFs: no undefined;
clang ELFs: `w arena_legacy_write`). Fixed with Arena 2's proposed design:
hook pointer owned by `rt.c`, set only by `arena_legacy_serial_enable()`
in `legacy_debug.c`; `rt.o` no longer references the symbol at all.
Verification: `nm -u` empty for **both** compilers' ordinary apps after
rebuild; host suite and ABI check re-run green; guest harness's
legacy-symbol refusal in `guest_c2.py` now trips on nothing. The debug
syscall remains reachable only through the explicit legacy opt-in.

## I-5 — Live stdin behavior → **FIXED (canonical-verified)**

Implemented as a narrow Desktop path over the existing capability-owned
native streams: a windowed stream app receives a pre-created console
window, keyboard bytes go only to the focused window's owning
AppInstance, and Ctrl-D closes that instance's stdin writer only after
already-queued bytes drain. `experiments/c-runtime/guest_stdin.py`
drives real QMP keyboard input to two simultaneously running signed C
apps and requires exact attribution/order, no delivery to the unfocused
instance, burst/backpressure handling, EOF line/byte counts, no-grant
refusal, and explicit ASTR-v1 refusal. GCC and Clang first passed with
exits `[58, 73, 73, 58]` in the 2026-10-25 reconstructed development
receipts, then passed again canonically at clean `d314d20…` after B1
was resolved and at corrected clean `5aacbc8…` after I-10
(`receipts/canonical-corrected-5aacbc8-2026-10-26/guest/stdin/`). This
is not a general shell or POSIX terminal layer.

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

## I-10 — ASTR v2 reserved-byte enforcement gaps → **FIXED (corrected canonical-verified)**

Review of the `d314d20…` candidate found that the implementations did
not completely enforce ADR-0111's requirement that all reserved bytes
be zero. Rust validated the typed header/ring reserved fields but did
not inspect page tail 3328–4095 and accepted some sub-page lengths;
C validated the header but not each ring's control bytes 12–63 or the
page tail. ADR-0111 was not weakened and the accepted layout,
capacity, version, ownership, memory ordering, and capability model
were preserved.

Corrected chain: `8c472a2…` strengthened production validation and
host tests; `d5874b7…` repaired only the test-generated legacy-v1
fixture transform; `5aacbc8…` applied formatting only to the new Rust
tests. Rust now requires exactly one 4096-byte page before any caller
memory is read, rejects sub-page/overlong lengths without inspecting
beyond the supplied range, checks and initializes the reserved tail,
and retains header/ring checks. C now checks all ring controls and
the tail under its documented complete-one-page bare-pointer contract.

Regression/sensitivity evidence: Rust and C tests probe both endpoints
of the header, every ring control, and the tail, with valid/restored
positive controls. Previous C logic fails exactly the eight newly
covered ring/tail endpoints; previous Rust logic fails exactly the
three new tests. Corrected results are Rust 27/27, C host 1721 PASS,
and all ten targeted GCC/Clang canonical guest invocations PASS at
clean `5aacbc8…` (see
`receipts/canonical-corrected-5aacbc8-2026-10-26/sensitivity/`,
`host/`, and `guest/`).

## Q-1 — Registered attempt-1 failures at corrected source → **RETAINED / NOT REPRODUCED**

This is a qualification-history observation, not a claim of a
root-caused product defect. Complete registered attempt 1 at clean
`5aacbc8…` failed 2/121 suites after 6543 s:
`test_m84_crash.py` (only its final policy/close recovery marker
conjunction; the preserved recovery serial lacked `packaged READY`
but showed `pkg: INELIGIBLE` and clean shutdown) and
`test_m9_client_death.py` (the full-suite log records a fatal-halt
classification; its original production serial was overwritten by the
following red-control suite's restored GREEN run). Focused reruns
passed (M84 276 s; M9 14 s), and a second complete registered run at
the same unchanged clean source passed all 121 suites in 6574 s.

No source change was made between registered attempts 1 and 2, so no
product fix or definitive root cause is claimed. The failed attempt is
retained at
`receipts/canonical-corrected-5aacbc8-2026-10-26/attempts/registered-attempt1-5aacbc8/`
for M/C review rather than discarded.

## Environment blockers

**B2 is resolved** by the exact pinned QEMU under an isolated glibc
(`BACKEND-RECOVERY.md`); the host system glibc was not replaced.
**B1 is resolved** by C's official Rust 1.97.0 components at
`02a20fc…`, installed into the private `/opt/rust/prefix` after part
and whole-archive hash verification. Canonical rebuild/qualification,
protected-identity reproduction, registered 121/121, strict Phase 15,
100/100 final-artifact stability, and fresh-archive verification were
first run at `d314d20…` and are indexed in
`receipts/canonical-2026-10-26/`. After I-10 changed the image bits,
the applicable host/static battery, all ten targeted guests, complete
registered regressions (attempt 2), strict Phase 15, corrected
100/100 stability, screenshots, and fresh-archive verification were
rerun at final tested source `5aacbc8…` and are indexed in
`receipts/canonical-corrected-5aacbc8-2026-10-26/`. The prior image's
stability receipt was not reused. Development-image results keep
their explicit non-qualifying label. Acceptance and merge remain M/C
decisions, not environment blockers and not consequences of these
runs.
