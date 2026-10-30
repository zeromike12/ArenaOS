# Phase-15 checks: historical pins vs the current candidate

Two different things are called "Phase 15", and they must not be mixed.

## Historical pinned-artifact gate

`PHASE15_STRICT_GATE=1` pins the artifacts Phase 15 qualified, by hash
(EFI `bbbc63df…`, ESP `631cc645…` in the strict gate's own table). That gate
asks one question: *are these the exact bytes Phase 15 signed off?* It is a
historical control. Its pins describe those bytes and nothing else.

Editing those pins to admit a new candidate would destroy the only record of
what Phase 15 actually qualified. **No pin was edited in this round.**

## Current-candidate behavioural coverage

`tools/test_m15_guest.py` takes the artifact under test explicitly
(`ARENA_PHASE15_EFI`, `ARENA_PHASE15_ESP`) and first proves the ESP really
contains the supplied EFI, then runs the FP/SIMD behavioural checks in the
guest under two CPU profiles:

| Run | CPU profile | AVX available | Result |
|---|---|---|---|
| `phase15-avx` | `max,+nx,+smep,+smap` | yes | **PASS** |
| `phase15-qemu64` | `qemu64,+nx,+smep,+smap` | no (OSXSAVE/XCR0 checked) | **PASS** |

Artifact under test: EFI
`bcd86866dbe7c15420eec958bff3024f66963d5d1dd39945513a58da750d5d83`,
ESP `fb76cfa8bc89d7d2…` (the ESP is rebuilt per build; its embedded EFI is
byte-identical, see `canonical-build/esp-embedded-efi.txt`).

Raw output: `m15-current-candidate.log`. Per-run serial logs:
`build/serial-phase15-avx.log`, `build/serial-phase15-qemu64.log`.

## Coverage limitation

The strict gate still cannot pass for this candidate, by design: it is a
historical pin, not a suitability test. Current-candidate coverage is the
behavioural run above. The two are reported separately and neither is used
to claim the other.
