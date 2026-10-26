# Phase 16 corrected canonical qualification receipts — 2026-10-26

Classification: **CANONICAL — CORRECTED QUALIFICATION CANDIDATE (not
acceptance).**

This is a new receipt set for the bounded ADR-0111 reserved-byte
correction requested after review of the earlier `d314d20…`
candidate. The earlier receipts in `../canonical-2026-10-26/` are
preserved unchanged as historical candidate evidence. They are not
overwritten, relabelled, or treated as evidence for the corrected
image.

Nothing in this directory accepts Phase 16, authorizes a merge or
release, or starts Phase 17. M remains final acceptance authority; C
remains advisor/independent reviewer.

## Tested source and correction chain

Final OS/test source for every corrected canonical run:
`5aacbc88c50d4565a7bb9f4075be27e40df27779` on
`arena/2b4ae1a5-arenaos`, clean at the start and end of the targeted,
registered, strict, and stability stages recorded here.

The source correction was committed before final qualification:

1. `8c472a2fe110b6bb3db81145342043de5ee75e03` — production and
   host-test enforcement of ASTR v2 reserved bytes in Rust and C.
2. `d5874b72f857261fe3c41eb7e7ac7d8af6431463` — harness-only repair
   to the test-generated legacy ASTR-v1 fixture transform.
3. `5aacbc88c50d4565a7bb9f4075be27e40df27779` — formatting-only
   `rustfmt` correction to two assertions in the new Rust tests; this
   is the final tested source.

### What changed

ADR-0111 was strengthened in implementations, not weakened. Layout,
capacity 1024, version 2, ownership, counters/memory ordering, and the
capability model are unchanged.

- Rust (`userspace/arena-runtime/src/streams.rs`):
  - `region_ptr` now requires exactly one 4096-byte page before any
    caller memory is dereferenced. Lengths 0, 3327, 3328, and 4095 are
    rejected as `TooSmall`; overlong lengths are also refused rather
    than silently treated as the one-page contract.
  - `attach` reads reserved tail bytes 3328–4095 only after that exact
    length gate and combines a nonzero tail with the existing header
    and each-ring reserved checks as `BadFormat`.
  - `initialize` explicitly zeroes bytes 3328–4095 after writing the
    typed region.
  - Tests probe both endpoints of header 16–63, each ring control's
    12–63 at page bases 64/1152/2240, and tail 3328–4095; include a
    valid/restored positive control; and cover the length boundaries
    and initializer zeroing above.
- C (`experiments/c-runtime/src/streams.c` and
  `include/arena/rt.h`):
  - `arena_ring_page_check` now checks header 16–63 (already present),
    every ring's control bytes 12–63, and tail 3328–4095.
  - Compile-time assertions pin the v2 tail offset to 3328 and require
    the rings plus tail to fit in one page.
  - The header documents the existing bare-pointer contract precisely:
    callers must supply one complete readable/writable-as-required,
    4096-byte, 4096-aligned page. The C API has no length parameter and
    cannot safely bounds-check a shorter object.
  - Host tests probe both endpoints of every reserved range on a
    complete page, with valid and restored positive controls.
- Legacy fixture (`experiments/c-runtime/guest_stdin.py` in commit
  `d5874b7…`): the generated v1-geometry copy now drops both v2-only
  static assertions. Production `streams.c` keeps both assertions
  unconditional.

Sensitivity evidence is in `sensitivity/`:

- Previous C production `streams.c` with the final host tests fails
  exactly 8/1721 checks: both endpoints in each of the three ring
  controls and both page-tail endpoints.
- Previous Rust production logic with the final tests gives 24 passes
  and exactly the three new tests failing (exit 101).
- Corrected production code passes Rust 27/27 and C host 1721 checks.

## Final artifacts and protected identities

Final pair, rebuilt once from clean `5aacbc8…` after registered
attempt 2 and then used unchanged for strict Phase 15 and stability:

- EFI (11,038,720 bytes):
  `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
- ESP (16,777,216 bytes):
  `04ffaf4681e92fe87886bdcff29b175df032ae9d88bd6cae59b34ad070ea674a`
- Protected Phase 14 PIE:
  `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`
- Protected signed Phase 14 APB1:
  `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`
- Kernel ISA audit in both canonical image builds: PASS over 120,911
  instructions.

Build path: physical bind mount `/workspace/ArenaOS` of
`/home/user/ArenaOS`. The targeted suites rebuild through canonical
`tools/build.sh --image`; EFI is invariant at `7dcee442…2bfe6`, while
each invocation's ESP differs because of FAT build metadata. Every
guest envelope records its exact ESP. No protected guard or expected
hash was changed.

## Environment

`environment.json` is the machine-readable identity record. In short:

- Rust/Cargo 1.97.0 from C's official hashed components, private
  prefix `/opt/rust/prefix`, installed with official `install.sh`.
  - none-target `libcore`:
    `6b75c5188dca46c917a67ade52dddea81460f96dc4732003cebea2e22a5b1b09`
  - UEFI-target `libcore`:
    `a7de36b0262b7d94cdcf1acb3f78288e7f5a57d546568b3ccaec27cd122c5494`
- GCC 12.2.0; Zig 0.17.0 (`zig cc` Clang driver).
- Locally modified QEMU 11.1.2, explicit TCG, binary
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`,
  wrapper
  `92ec1c12cc174d549dfdbedf921984784252d66630bdeca32d248085b42c76a1`,
  under isolated glibc 2.40. Host system glibc was not replaced.
  - isolated libc:
    `eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f`
  - isolated loader:
    `63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458`
- Preserved Phase 15 package firmware hashes are in
  `environment.json`.

Canonical environment shape:

```sh
export ARENA_RUST_BIN=/opt/rust/prefix/bin
export ARENA_QEMU=/home/user/phase16-backend/qemu-patched
export ARENA_OVMF_CODE=/home/user/phase15-pkg/firmware/OVMF_CODE_4M.fd
export ARENA_OVMF_VARS=/home/user/phase15-pkg/firmware/OVMF_VARS_4M.fd
export ARENA_ZIG=$HOME/.local/lib/python3.11/site-packages/ziglang/zig
export ARENA_GLIBC_DIR=$HOME/glibc-2.40
source tools/dev-env/env.sh
```

## Host/static battery

File: `host/host-canonical.log` — overall exit 0.

- Rust `arena-runtime --lib`: 27 passed, 0 failed.
- C runtime GCC and Clang guest-ELF builds and loader/ISA audits: PASS.
- C ABI cross-check: 92 constants, mismatches `[]`.
- C host suite: `LIBC-HOST checks=1099 failures=0`;
  `HOST-ONLY RESULT PASS (1721 checks)` under ASan+UBSan.
- Judge negative controls: C2 9/9, FP 12/12, Notification 6/6;
  C2 ISA-audit self-test PASS.
- FP app builds for GCC/Clang: 7-case image control PASS, audit PASS,
  host mutant detected.

The complete registered run separately exercises the repository's
`cargo fmt --check`, test, Clippy, and release-build blocks; see the
attempt history below for why that distinction matters.

## Targeted canonical guest suites

Directory: `guest/`; index: `guest/index.json`; process results:
`guest/results.tsv`. All ten invocations exited 0 from clean
`5aacbc8…`.

```sh
python3 -u experiments/c-runtime/guest_phase16_canonical.py \
  --suite native|c2|fp|stdin|notification -- --cc gcc|clang
```

| Suite | GCC | Clang | Semantic result |
|---|---:|---:|---|
| Native/C1 | PASS (35 s) | PASS (35 s) | 9/9 groups, 98 checks, exits `[58, 57]` |
| C2 multi-app | PASS (46 s) | PASS (44 s) | 9/9 console groups, 88 checks, xxHash digest exact, exits `[58, 58, 61, 63]` |
| FP | PASS (43 s) | PASS (40 s) | positive FP `66/0/0`; isolated mutant `66/19/0` detected; exits `[58, 71, 72]` |
| Live stdin | PASS (49 s) | PASS (47 s) | focused A/B attribution, 118/7 EOF byte counts, no-grant and ASTR-v1 refusals; exits `[58, 73, 73, 58]` |
| Notification | PASS (63 s) | PASS (64 s) | production sole candidate exit 75; restored-mutant two-candidate `NONE`, empty peek `-1005`, blocking `-1012`, exit 76 |

ASTR v2 is exercised by the C1 near-wrap checks, C2 stream groups,
and live stdin. The Notification ambiguity state remains unreachable
through production launchers found in the audit; the accepted
restored-mutation method is used and production restoration is
asserted by the suite.

## Attempts, failures, and retries

No failed attempt below is relabelled as a pass.

1. **Targeted attempt at `8c472a2…`:** protected build and host
   battery passed; 8/10 targeted guest invocations passed. Both
   live-stdin invocations failed before boot because the generated
   legacy-v1 fixture changed `RING_STRIDE` to 832 while retaining the
   production v2 tail-offset static assertion (`2560 == 3328`).
   Focused reproductions and results are in `attempts/`. Commit
   `d5874b7…` fixed only the generated fixture transform.
2. **Registered attempt at `d5874b7…`:** protected build, host
   battery, and all ten targeted guests passed. The complete
   `tools/run_tests.sh` then exited 1 after 8181 s with 1/121 suites
   failed; the sole failure was `cargo fmt --check` on two assertions
   in the new Rust tests. The original full external log was lost in a
   sandbox reset and is **not** represented as recovered. A focused
   exact reproduction is
   `attempts/registered-attempt1-d5874b7-rustfmt.log`. Commit
   `5aacbc8…` is the formatting-only correction.
3. **Registered attempt 1 at final source `5aacbc8…`:**
   `tools/run_tests.sh` exited 1 after 6543 s with 2/121 suites
   failed (`test_m84_crash.py`, `test_m9_client_death.py`). The full
   log and diagnostic serials are preserved under
   `attempts/registered-attempt1-5aacbc8/`.
   - M84 failed only the final policy/close recovery conjunction;
     its preserved recovery serial shows `pkg: INELIGIBLE` and clean
     shutdown but lacks `packaged READY`. No root cause beyond the
     observed marker conjunction is claimed.
   - M9's production serial was overwritten by the immediately
     following red-control suite's restored GREEN invocation; the
     full-suite log retains the original fatal-halt classification
     and assertion.
   - Focused reruns then passed (M84 in 276 s; M9 in 14 s), but those
     passes were not substituted for a complete green run.
4. **Registered attempt 2 at same clean `5aacbc8…`:** complete rerun
   passed in 6574 s; see `registered/full-suite.log` and
   `registered/run.json`.

## Registered regressions

Directory: `registered/`.

```sh
bash tools/run_tests.sh
```

Attempt 2 result: **ALL TESTS PASSED (121 test suites)**, runner exit
0, clean source at start/end. Intentional RED controls fail their
mutant boots as designed while their suites pass. The failed attempt 1
remains separately preserved and is part of the qualification history.

## Strict Phase 15

Directory: `strict-phase15/`.

```sh
ARENA_PHASE15_EFI=<efi> ARENA_PHASE15_ESP=<esp> \
  python3 tools/test_m15_guest.py
```

- Corrected final pair: PASS on `max` AVX-capable and `qemu64`
  profiles (14 s wrapper elapsed).
- Preserved accepted Phase 15 pair: PASS on both profiles (15 s).
- Preserved pair identities: EFI
  `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`;
  ESP
  `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.

Raw serials for all four boots are beside the checker logs.

## Final-artifact stability

Directory: `stability/`.

```sh
bash tools/stability_loop.sh 100
```

Result: **100/100 boots fully green, fail=0**, 942 s, on the exact
final EFI/ESP above. Receipt:
`7dcee442…2bfe6 100/100`. All 100 pixel-receipt texts/hashes are in
`pixel-receipt-hashes.json`. Boot-1 and boot-100 retained PPMs are
byte-identical (`223f56b9…0fe0`); screenshot 09 is a lossless PNG
conversion of boot 100.

The prior `d314d20…` 100/100 receipt was not reused.

## Screenshots

Corrected genuine guest captures are added beside, not over, the
historical captures:

- `../../screenshots/07-c-native-desktop-corrected-5aacbc8.png`
- `../../screenshots/08-live-stdin-corrected-5aacbc8.png`
- `../../screenshots/09-stability-boot-100-corrected-5aacbc8.png`

Captions 07–09 identify the exact run, source commit, EFI, applicable
ESP, backend, and linked receipts.

## Demonstration archive and fresh extraction

The sealed corrected archive and its external SHA-256 sidecar are
committed under `../../artifacts/`. Post-sealing verification receipts
are kept in `fresh-archive/` in this directory, outside the archive,
so recording the archive hash cannot mutate the artifact being
verified. See that sidecar and `../../FINAL-REPORT.md` for the exact
archive identity and fresh-extraction guest result.

## Remaining limits

- The emulator is the identified locally modified QEMU TCG backend,
  not upstream QEMU and not hardware.
- The ESP bytes vary across rebuilds; only the final ESP above is the
  byte-fixed member used for strict Phase 15, stability, packaging,
  and fresh extraction.
- Guest bundles use the repository RFC 8032 test-fixture signing path
  where summaries state it; production key management is out of scope.
- Notification ambiguity remains a restored test mutation because
  production launchers cannot currently emit that startup state.
- The C runtime remains the bounded ADR-0112 subset, not full
  POSIX/C11. B-GFX-1 remains a handoff decision; Phase 17 has not
  started.
