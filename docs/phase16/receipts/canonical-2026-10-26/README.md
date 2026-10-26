# Phase 16 canonical qualification-candidate receipts — 2026-10-26

Classification: **CANONICAL — QUALIFICATION CANDIDATE (not acceptance).**

These receipts are the B1-resolved rerun requested after C supplied the
official Rust 1.97.0 target components in commit
`02a20fce915bdd0bdbbc24dcdc2fa0b63a1cd7a1`. They are separate from the
2026-10-25 development receipts under
`../dev-guest-2026-10-25-reconstructed/`, which remain development
evidence only and are not relabelled.

Nothing in this directory declares Phase 16 accepted, authorizes a
merge or release, or starts Phase 17. M remains final acceptance
authority; C remains advisor/independent reviewer.

## Source and final artifacts

- OS/test source for every canonical run:
  `d314d204aa86ec4018659f3f211cd3a488d30d87` on
  `arena/2b4ae1a5-arenaos`, clean before and after the targeted guest
  runs and clean at both ends of the registered regression run.
- Build path: a physical bind mount at `/workspace/ArenaOS` (not a
  symlink). This was necessary for the protected Phase 14 byte
  identities; the failed home-path and symlink attempts are retained in
  `build/phase14-*.log`.
- Final EFI (11,038,720 bytes):
  `850cce0da36daa5d51452a4c4c4f01ae446b09272de293b3e14ad258f3ccaa79`
- Final ESP (16,777,216 bytes):
  `65b489016527a49e394ab6e7a57cb592045a3c80b55cceed7610aa448cb519d9`
- Protected Phase 14 PIE:
  `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`
- Protected signed Phase 14 APB1:
  `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`
- Kernel ISA audit in the final build: PASS over 120,911 instructions.

The targeted suites each invoke the canonical `tools/build.sh --image`
path through `mtest.build`. Their EFI is invariant at `850cce0d…aa79`;
their ESP hash varies between invocations because the FAT image bytes
include build-time metadata. Every suite summary records the exact ESP
it used. The final ESP above was generated once after the registered
run and was then used unchanged for strict Phase 15 and stability.

## Environment

`environment.json` is the machine-readable identity record. In short:

- Official component archives were reconstructed exactly as directed by
  `../../dependencies/rust-1.97.0/README.md`: all numbered parts passed
  `PARTS-SHA256SUMS`; both reconstructed archives passed `SHA256SUMS`;
  each archive's official `install.sh` installed into the private
  prefix `/opt/rust/prefix`.
  - `x86_64-unknown-none` archive:
    `5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a`
  - `x86_64-unknown-uefi` archive:
    `2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0`
  - Installed `libcore` hashes are recorded in `environment.json` and in
    every guest summary's `phase16_evidence.b1.components`.
- rustc/Cargo: Rust 1.97.0; GCC 12.2.0; Zig 0.17.0 (`zig cc` is the
  Clang driver used by the C-runtime harness).
- Emulator: locally modified QEMU 11.1.2, explicit TCG, binary SHA-256
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`,
  launched through an isolated glibc 2.40 runtime. The host system
  glibc was not replaced.
  - isolated `libc.so.6`:
    `eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f`
  - isolated `ld-linux-x86-64.so.2`:
    `63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458`
- Firmware: preserved Phase 15 package OVMF code/vars, hashes in
  `environment.json`.

Common environment for the guest commands below (paths are those of
the sandbox that produced the receipts):

```sh
export ARENA_RUST_BIN=/opt/rust/prefix/bin
export ARENA_QEMU=/home/user/phase16-backend/qemu-patched
export ARENA_OVMF_CODE=/home/user/phase15-pkg/firmware/OVMF_CODE_4M.fd
export ARENA_OVMF_VARS=/home/user/phase15-pkg/firmware/OVMF_VARS_4M.fd
export ARENA_ZIG=$HOME/.local/lib/python3.11/site-packages/ziglang/zig
export ARENA_GLIBC_DIR=$HOME/glibc-2.40
source tools/dev-env/env.sh
```

## Build and protected Phase 14 reproduction

Directory: `build/`

- `phase14-home-path-failed.log`,
  `phase14-baseline-home-path-failed.log`, and
  `phase14-baseline-symlink-failed.log` record the investigated
  non-reproductions. No guard or expected hash was changed.
- `phase14-baseline-bind-mount-pass.log`: pristine Phase 15 acceptance
  baseline `f103926…`, built at physical `/workspace/ArenaOS`,
  reproduced both protected identities.
- `phase14-current-bind-mount-pass.log`: current Phase 16 source did
  the same through unchanged `tools/build_phase14_pie.sh` and the
  canonical bundle path.
- `canonical-build-image-pre-targeted.log`: first clean `d314d20`
  `tools/build.sh --image` before targeted guest runs.
- `final-canonical-build.log`: final clean `d314d20` build after the
  registered run; this produced the final EFI/ESP above.

## Host/static canonical battery

File: `host/host-canonical.log` — overall exit 0. It contains:

- Rust `arena-runtime --lib`: 24 passed, 0 failed.
- C runtime GCC and Clang guest-ELF builds and loader/ISA audits: PASS.
- C ABI cross-check: 92 constants, mismatches `[]`.
- C host suite: `LIBC-HOST checks=1099 failures=0`;
  `HOST-ONLY RESULT PASS (1707 checks)` under ASan+UBSan.
- Judge negative controls: C2 9/9, FP 12/12, Notification 6/6,
  C2 ISA-audit self-test PASS.
- FP app builds for GCC/Clang: 7-case image control PASS, audit PASS,
  host mutant detected.

## Targeted canonical guest suites

Directory: `guest/`; index: `guest/index.json`; process results:
`guest/results.tsv` (all ten invocations exit 0).

Command shape:

```sh
python3 -u experiments/c-runtime/guest_phase16_canonical.py \
  --suite native|c2|fp|stdin|notification -- --cc gcc|clang
```

Unlike `guest_phase16_dev.py`, the canonical runner does **not**
substitute `mtest.build`; each unchanged suite builds through
`tools/build.sh`. Each summary is merged with a self-identifying
`phase16_evidence` envelope and has `evidence_class: GUEST-CANONICAL`,
clean source, B1/component identities, protected Phase 14 matches,
image/backend/firmware/compiler hashes, and the exact command.

| Suite | GCC | Clang | Semantic result |
|---|---:|---:|---|
| Native/C1 | PASS | PASS | 9/9 groups, 98 checks, exits `[58, 57]` |
| C2 multi-app | PASS | PASS | 9/9 console groups, 88 checks, xxHash digest exact, exits `[58, 58, 61, 63]` |
| FP | PASS | PASS | positive FP `66/0/0`; isolated mutant `66/19/0` detected; exits `[58, 71, 72]` |
| Live stdin | PASS | PASS | focused A/B attribution, 118/7 EOF byte counts, no-grant and ASTR-v1 refusals; exits `[58, 73, 73, 58]` |
| Notification | PASS | PASS | production sole candidate exit 75; two-candidate mutant `NONE`, empty peek `-1005`, blocking `-1012`, exit 76 |

ASTR v2 coverage is exercised through the C1 near-wrap checks, the C2
stream groups, and live stdin. The live-stdin suite also installs a
test-only ASTR-v1-layout application and requires explicit refusal:
v1 applications are not silently run.

Capability negatives in these targeted runs include no-stream exits,
live-stdin no-grant refusal, ASTR-v1 refusal, FP no-stream/mutant
controls, and Notification ambiguity. The Notification two-candidate
state cannot be produced by any production launcher path found in the
audit; `guest_notification.py` therefore follows the repository's
restored-mutation pattern. It first passes the production image, then
applies a one-count, explicitly labelled Desktop test fixture that adds
a distinct second role-Other/RW Notification descriptor, requires the
mutant marker exactly once, and finally asserts byte-exact restoration
of the Desktop source and production EFI/ESP/profile. The production
source at rest is unchanged.

Where an unsuffixed `bundle-identity.json` exists (`native`,
`notification`), it is the last compiler invocation's build identity;
both compilers' ELF/bundle identities are preserved in their
respective `summary-gcc.json` / `summary-clang.json` files.

## Registered regressions

Directory: `registered/` (`full-suite.log`, `run.json`).

```sh
bash tools/run_tests.sh
```

Result: **ALL TESTS PASSED (121 test suites)**, runner exit 0, source
commit `d314d20…` clean at start and end. Intentional RED controls in
the log fail their mutant boots as designed while their suites pass;
there were no semantic suite failures. Phase 13 installed-app
pressure, all Phase 14 PIE positive/negative/mutation suites, strict
`test_m15_guest.py`, and M1–M12/M8.x/M9 suites are included in this
registered count. The Phase 16 C-runtime guest suites are not part of
the historical registry and were run separately above.

## Strict Phase 15

Directory: `strict-phase15/`.

```sh
ARENA_PHASE15_EFI=<final-efi> ARENA_PHASE15_ESP=<final-esp> \
  python3 tools/test_m15_guest.py
ARENA_PHASE15_EFI=/home/user/phase15-pkg/artifacts/arena-boot.efi \
ARENA_PHASE15_ESP=/home/user/phase15-pkg/artifacts/arena-esp.img \
  python3 tools/test_m15_guest.py
```

Both the final Phase 16 pair and the preserved accepted Phase 15
package pair PASS on AVX-capable (`max`) and baseline (`qemu64`) CPU
profiles, including genuine #XF delivery, MMX, clean shutdown, and all
reported historical results. Raw serials for all four boots are kept
beside the two checker logs.

Preserved package control identities:

- EFI `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`
- ESP `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`

## Final-artifact stability

Directory: `stability/` (`run.json`, `stability-100.log`, receipt,
derived expectation, boot-100 serial, and hashes/text for all 100 QMP
pixel receipts).

```sh
bash tools/stability_loop.sh 100
```

Result: **100/100 boots fully green, fail=0**, 914 s, using the final
EFI/ESP above unchanged. The script receipt is
`850cce0d…aa79 100/100`. Boot-1 and boot-100 retained PPM frames are
byte-identical (`223f56b9…0fe0`); screenshot 06 is the PNG conversion
of boot 100.

## Demonstration package and fresh extraction

Sealed archive `ArenaOS-Phase16-Demonstration-d314d20.tar.gz`
(4,855,009 bytes), SHA-256
`08786f7900581b08e65acef7466c98d56e05587e6f873c46d2c1ed0cd22225fc`.
It contains the final artifacts, firmware, these receipts, screenshots,
submission docs, package manifest/checksums, a verifier, and a focused
tracked-file source subset of `d314d20…` (canonical C runtime plus the
strict checker/harness imports; the full tracked tree is not bundled
because it is about 314 MB with historical checkpoint archives).

Post-sealing sidecar: `fresh-archive/` (`run.json`, listing, logs, raw
serials). Attempt 1 failed before any guest boot because the long
extraction prefix made QEMU's vcon Unix socket path too long; the
package verifier/README were amended to fail closed on that condition,
the archive was resealed, and attempt 2 under a short prefix passed
static verification plus the extracted strict checker on both CPU
profiles. Extracted EFI/ESP hashes were unchanged after the guest run.

## Screenshots

- `../../screenshots/04-live-stdin-canonical-2026-10-26.png`
- `../../screenshots/05-c-native-desktop-canonical-2026-10-26.png`
- `../../screenshots/06-stability-boot-100-canonical-2026-10-26.png`

Captions 04–06 identify the exact run, source commit, EFI, ESP, and
linked receipt for each genuine guest capture.

## Remaining limits of this evidence

- The emulator is the specifically identified locally modified QEMU
  TCG backend, not upstream QEMU and not hardware.
- The C applications are signed with the repository's RFC 8032 test
  fixture key where the summaries say so; this proves the signed APB1
  path, not production key management.
- The Notification ambiguity boot is a restored test mutation because
  production launch paths intentionally cannot create that record. It
  is guest evidence for the runtime's refusal policy, not a claim that
  a production launcher emitted two candidates.
- ESP bytes are not reproducible across rebuilds (FAT metadata); the
  EFI is the invariant OS image. The final ESP identity above is the
  exact pair member used for strict Phase 15 and all 100 stability
  boots.
