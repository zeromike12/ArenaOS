# V1 test design — host/guest/negative evidence

**Status:** designed; blocked from guest execution by the missing ADR-0114
Identify implementation (ADR-0114 remains Proposed, not implemented in V1).
Host controls and static audits are runnable. Every guest case below is
written to run on the genuine QEMU baseline (pinned QEMU 11.1.2 TCG +
OVMF + current-main ESP) through the QMP-driven desktop interaction
pattern (`tools/qmp.py`, `tools/test_m10_apps.py` Desktop class:
`click`, `shot`, `serial`, `wait`, `type_text`), judged by the guest's own
exit receipt `EXIT_RE = r"^\[desktop\] child Process-cap exit status=(\d+)\r?$"`
plus PPM pixel assertions on genuine QEMU `screendump` captures. No case
counts setup/logging as verified behavior; every assertion is on
guest-observed output (serial receipt, surface pixels, focus events).

**Labeling rule:** QEMU `screendump` PPMs are guest captures. Any layout
sketch in this document is a mockup, labeled as such, never presented as OS
execution. Host/static results and guest results are kept distinctly
labeled; neither is presented as the other.

## 0. Harness contract reused from the codebase

- Guest under test: `c-gfx-app` (experiments/c-runtime/app/c_gfx_app.c),
  built with GCC and Clang, statically audited (ET_EXEC, no W+X, zero
  x87/MMX/SSE/AVX).
- Guest boot: desktop=True QEMU boot (pointer device, QMP socket at
  `build/qmp-{label}.sock`, serial on stdio chardev), signed APB1 bundle
  seeded into AFS2 `/Users/user/Desktop/`, installed via protected-path
  desktop icon click, launched through the real launcher.
- Exit judgment: exact exit sequence via EXIT_RE on the serial log; each
  test case names its expected code (0 = clean, 1..13 = distinct launch or
  teardown failures).
- Surface evidence: `shot(name)` PPM at 640×480 RGB888; pixel assertions
  computed on the PPM (not dimensions alone), e.g. pixel (x,y) at frame f
  equals the app's deterministic integer chequerboard formula.

## 1. Positive cases

### P-1 — Valid launch: render + input + animation + clean exit

1. Launch the app; expect discovery success (exactly one endpoint and one
   qualifying surface, kind+rights over the verified record) → Create
   640×480 → first frame published (`Damage` FULL).
2. `shot(t0)` → PPM matches the deterministic pattern at frame 0 (pixel
   assertion on every sampled pixel of a grid: 16×16 cells over the
   surface).
3. Wait for N animation frames (N ≥ 60, ~2 s at ~30 fps); `shot(t1)` → PPM
   differs from t0 at the predicted cells (animation proven by observable
   frame/state information: the deterministic chequerboard pattern at
   frame f is `((x >> 4) + (y >> 4) + f) & 1`, so each cell's expected
   color at each sampled frame is computable; not a static image and not
   frame-parity sampling).
4. QMP-inject one key press (`type_text` / QMP `input-send-event` key
   sequence) and one pointer event (absolute axes + button via
   `input-send-event`).
5. `shot(t2)` → visible acknowledgment: flashing border (inverted cell
   colors) at the predicted frame window; serial log line
   `[gfx] event type=1 events=1` (key) / type=2 (pointer) when stdout is
   granted. Input isolation is proven by instance-specific event counters:
   each instance tracks its own `events_received` and logs it; an
   unfocused instance may keep animating, so B's surface is not required
   to remain pixel-identical — B's event counter must not increment when
   A is focused.
6. Close: `DestroyWindow` → app exits; serial receipt code 0.
7. `shot(t3)` → the application's content region is no longer repainted
   (the Desktop has removed the window; the underlying desktop is restored
   appropriately). A frozen last frame is insufficient: the window must
   disappear from the desktop surface and resource accounting must settle.

Verdict: PASS iff every pixel assertion, the serial receipt code 0, the
Desktop focus transition (Focus event with b[29]=1 then 0 on close), the
window disappearance, and the instance-specific input counters all hold.

### P-2 — Descriptor reordering

Same as P-1, but the launcher's grant record lists the endpoint and
surface descriptors in a different order (and interleaved with other
role-OTHER descriptors). Discovery is by kind+rights, so behavior must be
byte-identical to P-1: exit 0, same pattern pixels, same input
acknowledgment.

Verdict: PASS iff P-1's assertions hold on the reordered grant. (Proves no
slot/order dependence.)

### P-3 — Two-instance focus and input isolation

1. Launch instance A and instance B concurrently; both reach the render
   loop (two surfaces, two endpoints, two badged sessions).
2. Focus A (desktop click on A's window at its icon coordinates); type a
   key sequence into A.
3. Assert: A's surface acknowledges (flash + event log line); B's instance-
   specific event counter does NOT increment between two consecutive
   `shot`s (B received no input: B's counter unchanged; B's surface may
   keep animating, so pixel-identity between B's shots is NOT required).
4. Focus B; type into B; assert B acknowledges and A does not change.
5. Close A → receipt 0 for A; close B → receipt 0 for B.
6. After both exits, launch C → C succeeds (slots retired, no stale
   session state).

Verdict: PASS iff input is delivered to exactly the focused instance,
neither instance can render into the other's surface (separate
SharedRegions, separate generations), and re-launch after double close
succeeds.

### P-4 — Repeated launch/close with bounded settlement

Launch→close→launch→close→launch→close the same app K times (K ≥ 10), each
launch from a fresh APB1 bundle seed. Each launch must reach the render
loop within a bounded wall time (e.g. ≤ 30 s after the install click;
settlement checked by `wait` on the first `[gfx] identified` serial line
or on the first surface damage frame), each close must produce receipt 0,
and between consecutive launches the Desktop must report no surface
leftover (shot of the desktop background unchanged). The K-th launch
passes P-1's pixel assertions (no accumulation, no generation exhaustion).

Verdict: PASS iff all K cycles meet their bounded settlement checks and
receipt 0.

## 2. Negative cases

Each negative case seeds a mutated grant/environment, launches, and
expects the app's distinct exit code WITHOUT any window created and WITHOUT
any Damage frame published (`shot` shows no app surface; serial receipt is
the code).

| Case | Setup | Expected |
|---|---|---|
| N-1 | No BadgedEndpoint granted | exit 2, no surface |
| N-2 | No SharedRegion surface granted | exit 4, no surface |
| N-3 | Two BadgedEndpoints with exact rights | exit 3, nothing guessed |
| N-4 | Two surfaces with exact rights | exit 5, nothing guessed |
| N-5 | Endpoint with wrong rights (e.g. WRITE only) | exit 6 |
| N-6 | Surface with wrong rights (e.g. RO) | exit 6 |
| N-7 | Surface smaller than 301 pages | exit 7 (valid boundary: 301 pages OK; 300 pages refused) |
| N-8 | Identify refused: wrong frame version from Desktop | Identify-phase: ADR-0114 not implemented in V1; required coverage when accepted |
| N-9 | Identify refused: malformed reply (bad magic/reserved) | Identify-phase: required coverage when accepted |
| N-10 | Identify refused: badge echo mismatch (spliced reply) | Identify-phase: required coverage when accepted |
| N-11 | Identify refused: generation mismatch (stale surface) | Identify-phase: required coverage when accepted |
| N-12 | Desktop process gone before Identify | Identify-phase: required coverage when accepted |
| N-13 | App sends malformed message (bad opcode/version) | refused, no service effect (Desktop-side audit) |
| N-14 | Unsupported protocol version | Identify-phase: the V1 app speaks only ADSK-v1; version-2 Identify is not sent; required coverage when accepted |

Negative cases use the restored-mutation pattern where the production path
cannot emit the state (e.g. Desktop-side refusals N-13 are exercised
through a test harness that speaks the wire directly and asserts the
typed refusal code, while the guest asserts it never acts on an
undecodable frame).

Identify-phase cases (N-8..N-12, N-14) are **required V1 coverage** once
ADR-0114 is accepted and the Desktop implements Identify. They are listed
here as blocked-by-ADR, not removed from V1's required coverage.

## 3. Screenshot geometry oracle

QMP `screendump` captures the entire desktop (800×600 in the V1 baseline),
not just the application's surface. To validate the application's content:

1. **Locate the content region:** account for window position, Desktop
   chrome, clipping, and screen dimensions. The application's surface is a
   sub-rectangle of the desktop PPM, not the full PPM.
2. **Distinguish formats:** PPM storage is RGB888 (3 bytes per pixel,
   row-major); the shared surface uses 32-bit u32 pixels (0x00RRGGBB, 4
   bytes per pixel). When comparing PPM samples against the surface
   pattern, convert the PPM RGB888 pixel at (x,y) to the expected u32
   value `0x00RRGGBB` and compare against the deterministic formula.
3. **Assert on pixels, not dimensions:** every visual claim is a PPM pixel
   assertion against the documented deterministic formula, never a
   dimension-only check.

## 4. Region bounds oracle

The V1 surface requires 301 pages: 1 offset page at 4096 + 300 u32-pixel
pages for 640×480 (1,228,800 bytes = exactly 300 pages).

- **Valid boundary:** a surface with exactly 301 pages → accepted
  (exit proceeds past the size check).
- **Too-small refusal:** a surface with 300 pages → exit 7
  (GFX_SURFACE_TOO_SMALL).

The obsolete 225-page threshold is replaced by the actual 301-page
requirement for the currently documented 640×480 layout.

## 5. Evidence and identities recorded per run

For every executed case: QEMU command line, emulator SHA, OVMF CODE/VARS
SHAs, ESP SHA, guest ELF SHA (gcc and clang builds), serial log, QEMU
stderr, each `shot` PPM with SHA-256, exit receipt code, wall-clock
settlement times. Attempt directories via ARENA_TEST_ATTEMPT_DIR so a
rerun can never overwrite a failing run's raw evidence (Q-1 guard).

## 6. What tests deliberately do NOT do

- No screenshot-dimension-only assertions (D-3): every visual claim is a
  PPM pixel assertion against the documented deterministic formula.
- No "we injected input" assertions without app-visible receipt (D-4):
  input is proved by the surface acknowledgment and/or the event log line.
- No mock-only runs (D-6 pattern): tests run on the genuine QEMU baseline
  or are recorded as blocked; `exit 2 for missing QEMU/baseline kernel`
  mock paths are not accepted evidence.
- No serial/debug-side channel for the app (D-5): the app never writes
  to the legacy serial sink; the exit receipt comes from the Desktop's
  Process cap.
- No pixel-identity requirement for an unfocused animating instance
  (D-3 correction): an unfocused instance may keep animating; input
  isolation is proven by instance-specific counters, not by requiring
  pixel-identical PPMs.
- No frame-parity animation proof (animation correction): animation is
  proven by observable frame/state information (the deterministic
  chequerboard at each sampled frame), not by assuming elapsed time proves
  a specific frame.

## 7. Execution gate

Host controls and static audits: runnable now (see `run.py build+audit+abi+host`).

Guest cases (P-1..P-4, N-1..N-14): blocked until M accepts ADR-0114, the
Desktop implements Identify, and `c_gfx_app` is packaged as a bootable guest
and launched through the real launcher. Until then this document is the test
design; nothing here is claimed as executed evidence. Identify-phase cases
are listed as blocked-by-ADR, not marked N/A to remove them from V1's
required coverage.