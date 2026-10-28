# V1 historical-code audit — arena/13f9221b-arenaos @ bd1c60fd

**Scope:** historical branch `arena/13f9221b-arenaos`, last reviewed commit
`bd1c60fd832afc5d6d5a5b3a8c8102220964e4ed` (fetched locally, NOT merged,
NOT used as a source of kernel/runtime/startup assumptions).
**Purpose:** produce a reuse/adapt/reject table so V1 starts from main
`d2bfd66` with eyes open about what the historical branch attempted and where
it is unsound for V1's constraints.
**Standing rule (user):** do not merge the whole branch; do not copy older
kernel/runtime/startup assumptions; avoid the listed historical harness defects.

## Reviewed material at bd1c60fd

| Item | What it was | Relevance to V1 |
|---|---|---|
| `docs/GRAPHICS-ABI-PROPOSAL.md` | ADSK-v2 wire-layout proposal with Options A/B and a SyncFence opcode | Defines the rejected Option A (slot/index-based) and Option B shapes; informational only for ADR-0114 |
| `G2.1-C-RUNTIME-HANDOFF.md` | C runtime handoff: malloc/free/calloc/realloc/_exit from Arena 2; SDL3 self-sufficient; FP/SIMD calling-convention shared blocker | SDL3 handoff context (V1 does not do SDL3); FP/SIMD blocker noted for the C contract |
| `test_upstream_sdl3_guest_mock.py` | 24-step mock guest lifecycle; exit 2 when QEMU/baseline kernel missing | Mock harness pattern only; V1 guest tests must run on genuine QEMU baseline |
| Historical `userspace/desktop/src/bin/desktop.rs` | `V2_ENDPOINT_SLOT=1`, `V2_SURFACE_SLOT=2`, `V2_CLOCK_SLOT=3` | **Reject as identification mechanism**: V1 forbids identifying graphical capabilities by slot (ADR-0114) |
| Main `userspace/desktop/src/app_client.rs` | `SERVICE_ENDPOINT=1`, `SURFACE=2`, `CLOCK=3`, `DIAGNOSTICS=4` | Same slot constants; retained only as cap *locations*, never identity |

## Reuse / adapt / reject table

| # | Historical artifact/assumption | V1 disposition | Reason |
|---|---|---|---|
| H-1 | Slot-based graphical identification (`V2_ENDPOINT_SLOT=1`/`V2_SURFACE_SLOT=2`) | **Reject** | B-GFX-1: never identify by slot, order, or guessed index; ADR-0114 identifies by kind/rights + Identify round-trip |
| H-2 | Historical desktop wire layout options (ADSK-v2 proposal, Options A/B) | **Reject for V1** | No wire/ARST change in V1 (ADR-0114: no kernel/ARST/packaging change; ADSK-v1 preserved byte-identical) |
| H-3 | SyncFence opcode concept | **Reject for V1** | Out of scope; V1's Identify is a single bounded synchronous IPC call |
| H-4 | C runtime: `malloc/free/calloc/realloc/_exit` from Arena 2 | **Adapt** | The canonical C runtime at `experiments/c-runtime/` already provides this subset; V1's C graphical app links it unchanged (integer-only flags, no FP/SIMD) |
| H-5 | SDL3 self-sufficient guest (mock lifecycle) | **Reject for V1** | SDL3 execution is a separate compatibility milestone, blocked by B-GFX-1 until the identification contract exists |
| H-6 | `test_upstream_sdl3_guest_mock.py` mock-driven 24-step flow | **Adapt as pattern only** | Structure (marker-paced steps, exit-code judging) is a useful pattern; V1 tests must not be mock-only and must run on the genuine QEMU baseline with pixel assertions |
| H-7 | Desktop session/badge dispatch (historical) | **Adapt** | ADR-0090 badge-authenticated sessions already exist in main; V1 builds on them via ADR-0114 Identify |

## Historical harness defects V1 must not repeat

| Defect | Where seen / described | V1 guard |
|---|---|---|
| D-1 | ESP/kernel mismatch — booting an ESP whose embedded kernel does not match the source under test | Baseline record states exactly which ESP (f54797f2-era Phase 15 artifact) is booted and that current-main d2bfd66 could not be rebuilt in this sandbox (no Rust toolchain); no claim that the booted guest is mainline source |
| D-2 | Strings/overrides used as FP/SIMD qualifiers | C guest built with the runtime's integer-only flags (`-mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387`); static audit disassembles and asserts zero x87/SSE instructions |
| D-3 | Screenshot dimensions asserted without pixel assertions | V1 screenshots are genuine QEMU `screendump` PPM captures with documented dims; any pattern claim is backed by PPM pixel inspection, not dims alone |
| D-4 | Input injection without proof the app received it | Input changes are judged by visible in-surface acknowledgment (damage repaint) plus serial exit receipts, never by "we sent keystrokes" alone |
| D-5 | Counting setup/logging as verified behavior | Pass/fail verdicts require the guest's own exit receipt (`[desktop] child Process-cap exit status=N`) and surface-level evidence; harness logging is not a behavior claim |

## Verdict

Nothing from arena/13f9221b @ bd1c60fd is adopted as implementation. The branch supplies
pattern references (mock guest flow, C runtime subset, SDL3 handoff context) and a
negative example (slot-based identification), all of which V1 handles through ADR-0114
and the canonical `experiments/c-runtime/` runtime instead.
