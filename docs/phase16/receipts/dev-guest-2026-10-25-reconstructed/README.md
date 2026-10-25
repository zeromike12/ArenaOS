# Phase 16 reconstructed development evidence — 2026-10-25

**Classification: DEVELOPMENT ONLY — NOT QUALIFICATION EVIDENCE.**

These receipts were produced after the ASTR v2, FP-tightening, live-stdin,
and evidence-envelope reconstruction was restored onto pushed commit
`87987d45c156c5b43b631fd8a6c913bbd84a761a`. The guest runs themselves were
executed from that commit plus the dirty paths recorded in each summary's
`phase16_evidence` envelope. The same source was then committed without
further source changes as:

- `d5b8c0006bb755358dcf40f6b9cd59e32cba79ea` — Implement ASTR v2 stream rings
- `1513ba22e1d4822e3980c79b0e5c27949383af20` — Tighten FP guest probe and judge
- `0cfb6de5b583acf69ff0db203568223bfd05508f` — Add focused live stdin console windows
- `faccf879f9c67e276c01b9d0f46bc595ee3b2bb2` — Add self-identifying Phase 16 development evidence
- `a7ad4c284fc9c610a8a32226c3b86a182ff1d8bf` — Synchronize Phase 13 search launches

Blocker **B1 remains open**: the official Rust 1.97.0 target components are
unavailable in this sandbox, so the kernel/ESP below came from
`tools/build_phase16_dev.sh --image`. Nothing in this directory qualifies
Phase 16, accepts an implementation, or authorizes a merge.

## Exact development image and backend

| Artifact | SHA-256 |
|---|---|
| Development EFI `build/arena-boot.efi` | `cd32caf4816247ad99862bd548d0248c5e7d409e8f119ef93fc84d56809219c9` |
| Development ESP `build/arena-esp.img` | `0524e6c3fb51b8438a3657b41fd2bd5e2505966b3bfbefcbc03c41e0d4144513` |
| Protected Phase 14 fixture | `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e` |
| Pinned patched QEMU 11.1.2 binary | `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff` |
| Isolated glibc `libc.so.6` | `eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f` |
| Isolated glibc `ld-linux-x86-64.so.2` | `63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458` |
| OVMF code | `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94` |
| OVMF vars template | `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e` |

The development build completed with the kernel ISA audit and its negative
controls passing over 120,503 decoded instructions; see
`host/development-build.log`.

Environment used for every guest run:

```sh
source tools/dev-env/env.sh
export ARENA_QEMU=/home/user/phase16-backend/qemu-patched
export ARENA_OVMF_CODE=/home/user/phase15-pkg/firmware/OVMF_CODE_4M.fd
export ARENA_OVMF_VARS=/home/user/phase15-pkg/firmware/OVMF_VARS_4M.fd
export ARENA_ZIG=$HOME/.local/lib/python3.11/site-packages/ziglang/zig
export ARENA_GLIBC_DIR=/home/user/glibc-2.40
```

## Results in this directory

### Host

- Rust `arena-runtime --lib`: **24 passed, 0 failed**.
- C runtime host: **LIBC-HOST 1099 checks, 0 failures; HOST-ONLY 1707 PASS**.
- C ABI oracle: **92 constants, 0 mismatches**.
- FP build/audit: GCC and Clang PASS, required SSE/x87 present, AVX = 0,
  no undefined symbols; audit and 7-case FP-image controls PASS.
- FP judge self-test: **12/12 PASS**.
- C2 judge self-test: **9/9 PASS**.

Raw host logs are under `host/`.

### Guest C-runtime suites

Each suite was run once per compiler through
`experiments/c-runtime/guest_phase16_dev.py` against the exact ESP above.
The per-compiler summaries contain the self-identifying
`phase16_evidence` envelope; raw serials are beside them.

| Suite | GCC | Clang | Exact semantic result |
|---|---|---|---|
| C1 native | PASS | PASS | 9/9 groups, 98 checks, exits `[58, 57]` |
| C2 multi-app | PASS | PASS | console 9/9 groups / 88 checks, xxHash digest OK, exits `[58, 58, 61, 63]` |
| FP | PASS | PASS | positive `PASS checks=66 failed=0 skips=0`; mutant `FAIL checks=66 failed=19 skips=0`; exits `[58, 71, 72]` |
| Live stdin | PASS | PASS | exits `[58, 73, 73, 58]`; A EOF bytes 118, B EOF bytes 7; isolation and legacy-ASTR-v1 refusal included |

Commands were of the form:

```sh
python3 experiments/c-runtime/guest_phase16_dev.py \
  --suite native|c2|fp|stdin --esp build/arena-esp.img -- --cc gcc|clang --runs 1
```

### Desktop and Phase 15 regressions

- Phase 13 registry guest suite: **PASS** on the development ESP. It
  includes the exact native-stdin marker, all extra-window closes, five
  installed instances, 26 mixed AppInstances / 32 windows, exact teardown,
  and the next-boot favorites reload.
- M10 apps guest suite: **PASS** on the development ESP, including exact
  resource cleanup and durable editor/preferences checks.
- Phase 15 `test_m15_guest.py` on the development EFI/ESP: **PASS** for
  both `max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`.
- Phase 15 `test_m15_guest.py` on the unchanged package artifacts
  (`bbbc63df…238e` EFI / `631cc645…866f` ESP): **PASS** for both CPU
  profiles. This verifies the recovered emulator/runtime combination; it
  does not qualify the Phase 16 development kernel.

Raw host and serial logs are under `regressions/`.

## Screenshot

`../../screenshots/03-live-stdin-dev-2026-10-25.png` is a genuine QEMU
screendump converted from the live-stdin suite, SHA-256
`7b1b33e6acddd80850a17eb4b9ae500348be047893b83eac5449bfc27c1f0656`.
It shows the two separately launched input-probe console windows with
Marina focused. The C apps' byte-level output is not rendered inside those
surfaces; the semantic input/output/EOF proof is in `stdin/summary-*.json`
and the raw stdin serial logs.

## Failure found and fixed during this rerun

The first tightened FP GCC guest run produced the exact positive/mutant
verdicts and exits but the host judge rejected it because its blanket
`SKIP` substring test also matched the unrelated boot line
`configup: SKIP (no trusted test intent; no SET)`. The judge now rejects
only `[c-fp] … SKIP` lines, and its self-test covers both that unrelated
boot line (must pass) and an FP-app skip line (must fail). The FP guest
suites were then rerun; the receipts here are from the corrected judge.

Historical receipts elsewhere in `docs/phase16/receipts/` are unchanged.
This directory supersedes their stale-image development results for the
reconstructed source, but it does not erase or reclassify them.
