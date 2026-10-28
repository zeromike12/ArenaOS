# V1 application design — signed native C graphical client

**Status:** scaffolding source only; the Identify round-trip is mandatory
but inactive until M accepts ADR-0114 and the Desktop service implements the
Identify opcode. Until then the app fails closed at identification — it never
guesses which endpoint or surface is its own.
**Milestone:** V1 — Native Graphical Launch Foundation
**Built with:** `experiments/c-runtime/` canonical runtime, GCC and Clang
(`run.py build --cc gcc --cc clang`), audited with `run.py audit`.

## 1. Contract the app relies on (nothing new)

- ARST v2 startup record, verified by `arena_startup_gate` before `main()`:
  `arena_startup()->caps[]` gives each descriptor's slot/kind/role/rights.
- Capability kinds/rights from `userspace/abi.rs` / `arena/abi.h`:
  `CAP_KIND_BADGED_ENDPOINT=12`, `CAP_KIND_SHARED_REGION=7`, role OTHER=5,
  RIGHT_READ=1, RIGHT_WRITE=2, RIGHT_COPY=4, RIGHT_DESTROY=8.
- ADR-0090 grant profile: the trusted launcher gives this session exactly
  one BadgedEndpoint (WRITE|COPY|DESTROY) and exactly one writable
  session SharedRegion (READ|WRITE|COPY), both role OTHER.
- ADR-0114: the app locates them by kind/rights (uniqueness required),
  then runs a versioned Identify round-trip over the endpoint; the reply
  manufactures no authority — the app cross-checks the named surface
  (kind/rights/generation via SYS_CAP_DESCRIBE) against caps it already
  holds.
- ADSK wire (`userspace/desktop/src/wire.rs`): 64-byte frames, magic "ADSK",
  opcodes Create=1, Damage=2, Poll=3, DestroyWindow=11; reply bytes are
  written back into the same inline buffer.
- ADR-0114 Identify extension (ADSK version 2, opcode 13), frame layout:
  request: `b[4]=2, b[5]=13, b[8..10]=client version u16 LE (=1)`, rest zero;
  reply: `b[4]=2, b[5]=13, b[6]=status, b[8..16]=session serial u64 LE,
  b[16]=surface kind, b[17]=rights, b[18..26]=surface generation u64 LE`.
- SYS_CAP_DESCRIBE (29): `[kind, generation/id, rights]` for a held slot.
- SYS_SHARED_PAGES (52) / SYS_SHARED_MAP (37): size and map a SharedRegion.
- SYS_IPC_CALL (7): endpoint slot, two descriptor words, transferred cap,
  3-word out array, inline frame buffer — same shape as
  `userspace/desktop/src/app_client.rs::exchange`.
- `arena_exit(status)`: ends the process; the Desktop records the exit
  status through its held Process cap → serial `[desktop] child Process-cap
  exit status=N`.
- No syscall additions, no kernel change, no ARST change, no slot-based
  identification anywhere in the app.

## 2. Surface geometry (V1 convention)

640×480 RGB888 (921,600 bytes = exactly 225 pages), within the wire's
80×60..1024×768 bounds. The granted surface region must provide at least
225 pages (SYS_SHARED_PAGES); all framebuffer writes are bounded to the
convention rectangle. Geometry is a documented V1 convention, not an
identity mechanism: a mismatched region is a launch refusal, never a guess.

## 3. Startup sequence

1. `s = arena_startup();` → `!s->present` = exit 1.
2. Scan `s->caps[0..cap_count)`:
   - endpoint candidates: kind 12, role 5, rights == WRITE|COPY|DESTROY (14).
   - surface candidates: kind 7, role 5, rights == READ|WRITE|COPY (7).
   - zero candidates → exit 2 (no endpoint) / 4 (no surface).
   - two or more → exit 3 (ambiguous endpoint) / 5 (ambiguous surface).
   - rights mismatch → exit 6.
   Slot numbers are recorded only as IPC operands; never used to identify.
3. Learn the endpoint's own badge: `SYS_ENDPOINT_BADGE(endpoint_slot,
   endpoint_slot)` → `own_badge` (used to authenticate the Identify reply's
   session serial echo).
4. `SYS_CAP_DESCRIBE(surface_slot)` → require kind 7, rights 7; record
   `surface_generation`. `SYS_SHARED_PAGES(surface_slot) >= 225` else exit 7.
5. `SYS_SHARED_MAP(surface_slot, pages_needed)` → framebuffer VA; unmap on
   any later failure.
6. **Identify round-trip** (ADSK v2 opcode 13) over the endpoint slot:
   - encode request, `SYS_IPC_CALL(endpoint, 0, 0, CAP_NONE, out, frame)`;
   - retry only kernel STATUS_BUSY with the runtime's bounded backoff,
     then fail; a service status is never retried.
   - require reply magic/version/opcode, status 0, reserved bytes zero;
   - echo check: `reply[8..16] == own_badge` else exit 9 (spliced reply);
   - surface cross-check: among this process's held caps exactly one
     SharedRegion with kind 7/rights 7 whose `SYS_CAP_DESCRIBE` generation
     equals `reply[18..26]` else exit 9 (stale/foreign surface).
   - Any other shape → exit 9 (protocol failure, no graphical state).
   On every failure the app has created no window and mapped nothing it
   does not unmap — fail closed, no state change visible to Desktop.
7. `Create {640, 480}` over the endpoint → window handle in reply word `out[1]`.
   Create failure → exit 10.

## 4. Render loop (bounded)

For `frame` in `0..GFX_FRAMES` (300 frames, ~30 fps via `arena_busy_sleep_us`):

- Render deterministic integer pattern into the framebuffer:
  chequerboard of 16×16 cells with colors derived from `(cell_x + cell_y + frame)`
  using only integer arithmetic (no x87/SSE/AVX anywhere — enforced by the
  build flags and the static audit).
- If an input event was acknowledged this frame, draw a flashing border
  (inverted cell colors) for `APP_FLASH_FRAMES` frames — visible
  acknowledgment of real input.
- `Damage {handle, FULL}` to publish the surface.
- `Poll {handle}`; decode any Event frame in the reply buffer:
  - Focus (type 4, b[29] 0/1) → track focus state (used by the two-instance
    isolation test: input received only while focused).
  - Key (type 1) / Pointer (type 2) → `flash = APP_FLASH_FRAMES`,
    `events_received++`, log line via `arena_write(1, ...)` when stdio is
    granted (best-effort; missing stdio never fails the app — no serial
    fallback).
  - Close (type 3) → jump to teardown early.
  - Anything undecodable → continue (a spurious frame never changes state).

## 5. Teardown and exit

`DestroyWindow {handle}` over the endpoint, unmap the surface, then
`arena_exit(0)`. The Desktop's Process-cap receipt logs
`[desktop] child Process-cap exit status=0`. Any earlier failure exits with
its distinct code (1..10) so the serial receipt is unambiguous evidence.

## 6. Negative behaviors exercised by design (tests in docs/v1/03-test-design.md)

- Missing endpoint/surface → exit 2/4, no window created.
- Duplicate endpoint/surface → exit 3/5 (nothing guessed).
- Wrong rights on either cap → exit 6.
- Surface too small for the V1 geometry → exit 7.
- Identify refused (wrong version, bad magic, nonzero status, reserved
  bytes, badge echo mismatch, generation mismatch, zero/two matching
  surfaces) → exit 9, no graphical state.
- Desktop process gone → `STATUS_SERVICE_GONE` propagates, exit 9.
- Each failure leaves the process's other granted caps untouched and creates
  no window; a re-launch starts from a clean record.

## 7. What is deliberately NOT in this app

- No slot/ordinal-based identification (rejected historical pattern).
- No "ask an arbitrary endpoint who it is" — the endpoint called is the one
  the launcher granted; Desktop authenticates the caller via the kernel
  badge.
- No serial/debug output path, no privileges requested beyond the grant,
- no FP/SIMD instructions, no allocator beyond the runtime's buddy heap,
- no alternative startup layer, no security bypass.
