# V1 application design — signed native C graphical client

**Status:** implemented source (`experiments/c-runtime/app/c_gfx_app.c`),
built with GCC and Clang (`run.py build --cc gcc --cc clang`), audited with
`run.py audit` (zero x87/MMX/SSE/AVX, ET_EXEC, no W+X). The QEMU baseline is
established (current-main boots cleanly through genuine QEMU 11.1.2 TCG, see
docs/v1/00-baseline-record.md §6). Guest execution is blocked on the
ADR-0114 Identify round-trip (still Proposed, not implemented in V1). The
app's discovery-only path is diagnostic scaffolding: it locates candidates by
kind+rights over the verified record but does not resolve B-GFX-1, does not
implement an accepted identification contract, and does not satisfy V1's
positive acceptance case. **Milestone:** V1 — Native Graphical Launch
Foundation
**Built with:** `experiments/c-runtime/` canonical runtime, GCC and Clang.

## 1. Contract the app relies on (nothing new)

- ARST v2 startup record, verified by `arena_startup_gate` before `main()`:
  `arena_startup()->caps[]` gives each descriptor's slot/kind/role/rights.
- Capability kinds/rights from `userspace/abi.rs` / `arena/abi.h`:
  `CAP_KIND_BADGED_ENDPOINT=12`, `CAP_KIND_SHARED_REGION=7`, role OTHER=5,
  RIGHT_READ=1, RIGHT_WRITE=2, RIGHT_COPY=4, RIGHT_DESTROY=8.
- ADR-0090 grant profile: the trusted launcher gives this session exactly
  one BadgedEndpoint (WRITE|COPY|DESTROY) and exactly one writable session
  SharedRegion (READ|WRITE|COPY), both role OTHER.
- ADR-0114 (Proposed): the V1 app locates them by kind/rights over the
  verified record — uniqueness required, ambiguity refused, slots used only as
  IPC operands, never as identity. The ADR-0114 Identify round-trip
  (ADSK version 2, opcode 13) is specified in the ADR but NOT implemented in
  V1 while the ADR remains Proposed: the app never sends opcode 13, never
  queries `SYS_ENDPOINT_BADGE` (serve-side only), and never guesses which
  endpoint or surface is its own.
- ADSK wire (`userspace/desktop/src/wire.rs`): 64-byte frames, magic "ADSK",
  opcodes Create=1, Damage=2, Poll=3, DestroyWindow=11; reply bytes are
  written back into the same inline buffer. The Desktop authenticates the
  caller through the endpoint's kernel badge and the landed surface cap
  (kind 7, this session's object id, READ|WRITE|COPY).
- SYS_CAP_DESCRIBE (29): `[kind, object_id, rights]` for a held slot — object
  ids are descriptive, never authority.
- SYS_SHARED_PAGES (52) / SYS_SHARED_MAP (37, `(slot, writable_flag 0|1)`):
  size and map a SharedRegion; any writable flag other than 0 or 1 is
  rejected by the kernel.
- SYS_IPC_CALL (7): endpoint slot, two descriptor words, transferred cap
  slot, 3-word out array, inline frame buffer — same shape as
  `userspace/desktop/src/app_client.rs::exchange`. Blocking, no client-side
  deadline; only kernel `STATUS_BUSY` retries are bounded.
- `arena_exit(status)`: ends the process; the Desktop records the exit
  status through its held Process cap → serial
  `[desktop] child Process-cap exit status=N`.
- No syscall additions, no kernel change, no ARST change, no slot-based
  identification anywhere in the app, no `SYS_ENDPOINT_BADGE` client call,
  no capability transfer in V1.

## 2. Surface geometry (V1 convention)

640×480 with **u32 pixels** (0x00RRGGBB) = 1,228,800 bytes = exactly 300
pages, placed at pixel offset 4096 (one page) inside the granted
SharedRegion: the region must provide at least 301 pages
(SYS_SHARED_PAGES ≥ 301). All framebuffer writes are bounded to the
convention rectangle. Geometry is a documented V1 convention, not an
identity mechanism: a mismatched region is a launch refusal, never a guess.

## 3. Startup sequence

1. `s = arena_startup();` → `!s->present` = exit 1 (GFX_NO_STARTUP).
2. Scan `s->caps[0..cap_count)`:
   - endpoint candidates: kind 12, role 5, rights == WRITE|COPY|DESTROY (14).
   - surface candidates: kind 7, role 5, rights including READ|WRITE|COPY (7).
   - zero candidates → exit 2 (no endpoint) / 4 (no surface).
   - two or more → exit 3 (ambiguous endpoint) / 5 (ambiguous surface).
   - candidates exist but none qualifies → exit 6 (wrong rights).
   Slot numbers are recorded only as IPC operands; never used to identify.
3. `SYS_CAP_DESCRIBE(surface_slot)` → require kind 7, rights including
   READ|WRITE|COPY (else exit 6).
4. `SYS_SHARED_PAGES(surface_slot) >= 301` else exit 7 (surface too small).
5. `SYS_SHARED_MAP(surface_slot, 1)` → framebuffer VA (writable flag 1; any
   other value is rejected) else exit 8 (map failed). u32 pixel plane at
   `framebuffer + 4096`. Unmap on any later failure.
6. `Create {640, 480}` over the endpoint, landing the surface cap → window
   handle in reply word `out[1]` (else exit 9, create failed).
7. Render loop (section 4), then teardown (section 5). Any failure exits with
   its distinct code; the Desktop's Process-cap receipt logs
   `[desktop] child Process-cap exit status=N`.

## 4. Render loop (bounded)

For `frame` in `0..GFX_FRAMES` (300 frames, ~30 fps via
`arena_busy_sleep_us`):

- Render a deterministic integer pattern into the u32 pixel plane: a
  16×16-cell chequerboard with colors derived from
  `(cell_x + cell_y + frame)` using only integer arithmetic (no x87/SSE/AVX
  anywhere — enforced by the build flags and the static audit).
- If an input event was acknowledged this frame, draw a flashing border
  (inverted cell colors) for `GFX_FLASH_FRAMES` frames — visible
  acknowledgment of real input.
- `Damage {handle, FULL}` to publish the surface (failure → exit 10).
- `Poll {handle}`; decode any Event frame in the reply buffer:
  - Focus (type 4, b[29] 0/1) → track focus state.
  - Key (type 1) / Pointer (type 2) → `flash = GFX_FLASH_FRAMES`,
    `events_received++`, log line via `arena_write(1, ...)` when stdio is
    granted (best-effort; missing stdio never fails the app — no serial
    fallback).
  - Close (type 3) → jump to teardown early.
  - Anything undecodable → continue (a spurious frame never changes state).
- Event frames are validated (magic, version 1, opcode 5, echoed handle,
  reserved bytes zero); an unexpected landed capability is destroyed and the
  call refused.

## 5. Teardown and exit

`DestroyWindow {handle}` over the endpoint, then unmap the surface, then
`arena_exit(code)`. Teardown policy:

- Preserve an existing primary failure (`exit_code`); it is never
  overwritten by a cleanup failure.
- Attempt necessary cleanup regardless of the primary failure.
- If the operation was otherwise successful but required cleanup fails,
  return a distinct failure: `GFX_DESTROY_FAILED` (12) when
  `DestroyWindow` refuses, `GFX_UNMAP_FAILED` (13) when `SYS_SHARED_UNMAP`
  refuses.
- Record cleanup failures (log lines) even when preserving an earlier
  primary failure.
- Do not report clean teardown merely because process exit may eventually
  reclaim resources: a cleanup failure is a distinct nonzero exit.

The Desktop's Process-cap receipt logs `[desktop] child Process-cap exit status=N`.

## 6. Negative behaviors exercised by design (tests in docs/v1/03-test-design.md)

- Missing endpoint/surface → exit 2/4, no window created.
- Duplicate endpoint/surface → exit 3/5 (nothing guessed).
- Wrong rights on either cap → exit 6.
- Surface too small for the V1 geometry → exit 7.
- Map refusal → exit 8.
- Create refusal → exit 9, no graphical state.
- Damage refusal → exit 10.
- Poll refusal → exit 11.
- `DestroyWindow` refusal on a successful run → exit 12.
- `SYS_SHARED_UNMAP` refusal on a successful run → exit 13.
- Primary failure + cleanup failure → primary code preserved, cleanup
  failure logged.
- Each failure leaves the process's other granted caps untouched and creates
  no window; a re-launch starts from a clean record.

## 7. What is deliberately NOT in this app

- No slot/ordinal-based identification (rejected historical pattern).
- No `SYS_ENDPOINT_BADGE` client query (serve-side only).
- No Identify round-trip (ADR-0114 Proposed; not implemented in V1).
- No capability transfer in V1.
- No "ask an arbitrary endpoint who it is" — the endpoint called is the one
  the launcher granted; Desktop authenticates the caller via the kernel
  badge.
- No serial/debug output path, no privileges requested beyond the grant,
- no FP/SIMD instructions, no allocator beyond the runtime's buddy heap,
- no alternative startup layer, no security bypass.
