# Phase 16 baseline record (2026-10-24)

## 1. Source baseline

| Item | Identity |
|---|---|
| Assignment-expected `main` | `f103926d6b28d546fd6a9a32d2bf5f63a16d618b` (merge PR #4) |
| Actual starting HEAD | `f103926d6b28d546fd6a9a32d2bf5f63a16d618b` — exact match, clean tree, no newer commits on `origin/main` after fetch |
| Working branch | `arena/2b4ae1a5-arenaos` (platform-pinned; see `PLAN.md` §0) |
| C runtime source | `arena/f89ea987-arenaos` @ `84392b8f32a0b1ce26364926185723c7bbf7a3f3`; evidence of record `9c93f8c…` (docs-only commits after it, per Arena 2's own diff check) |
| Registered regression at Phase 15 acceptance | 121/121 suites (`docs/phase15/ACCEPTANCE.md`); to be recounted at Phase 16 qualification |

## 2. Environment prepared in this sandbox

Prepared with the repo's own `tools/dev-env/bootstrap.sh` (exit 0) plus one
addition; nothing downloaded was committed to Git.

| Component | Version / identity | Source |
|---|---|---|
| Rust | 1.97.0 (`rustc 1.97.0 2d8144b78`, `cargo 1.97.0 c980f4866`) | PyPI `arena-rust-toolchain==1.97.0` (bootstrap pin) |
| Bare-metal sysroots | built from `rust-lang/rust` tag `1.97.0` by `tools/build-sysroot.sh` | GitHub (bootstrap) — see blocker B1 |
| QEMU (workhorse) | 11.0.2 static musl + bundled EDK2 | npm `qemu-portable-linux-x64-musl@0.2.1` (bootstrap pin) |
| musl loader | 1.2.5 | GitHub (bootstrap) |
| GCC | 12.2.0 (Debian 12) | preinstalled |
| Clang | 22.1.8 via Zig 0.17.0 (`zig cc`) | PyPI `ziglang==0.17.0` (same Zig as Arena 2); `ARENA_ZIG=~/.local/lib/python3.11/site-packages/ziglang/zig` |
| Python | 3.11 + `pyfatfs`, `zstandard` | PyPI (bootstrap) |
| Accepted backend binary | patched QEMU 11.1.2, SHA-256 `e0ea2b1f…f26ff` (matches `docs/phase15/ACCEPTANCE.md`) | extracted from `docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz` — see blocker B2 |

Reproduce: `bash tools/dev-env/bootstrap.sh && pip3 install --user ziglang==0.17.0`,
then `source tools/dev-env/env.sh`. Build/launch: `tools/build.sh --image`,
`tools/run.sh`, `tools/run_tests.sh` (see `docs/RUNNING.md`).

## 3. Baseline smoke (before any Phase 16 edit)

Boot 1 — accepted Phase 15 strict image (ESP SHA-256 `631cc645…866f`,
EFI `bbbc63df…238e`, both matching ACCEPTANCE) on **unpatched** npm QEMU
11.0.2, `-accel tcg`, `qemu64,+nx,+smep,+smap`. Raw log:
`receipts/baseline-strict-serial-unpatched-qemu.txt`.

- `m1 8/8`, `m2 21/21`, `m3 13/13`, `m4 9/9` PASS; `m15: RESULT FAIL (5/6)`:
  `xf_delivery_probe` failed with post-MXCSR `0x1f01` instead of vector 19 —
  **exactly** the documented unpatched-TCG behavior (ADR-0113/QUALIFICATION).
  `#MF`/`#UD` containment and MMX state passed. This confirms the sandbox
  workhorse does not inherit the pinned backend's qualification.

Boot 2 — Phase 14 complete checkpoint ESP (EFI SHA-256 `418c63ac…ff2f`,
matching the archive's `sha256sums.txt`) with the full harness device set.
Desktop presented a real frame; serial shows m1–m12 suites PASS and the
`arena>` shell. Screenshot:
`screenshots/01-baseline-desktop-phase14esp.png` (800×600 QMP screendump
of the guest framebuffer, SHA-256 `dda4ff50…4153`). Note this is the
Phase 14 graphical image used as the meaningful desktop view; the Phase 15
strict image is headless by design, so its evidence is the serial log above.

## 4. Environment blockers (honest, unresolved)

- **B1 — protected Phase 14 fixture identity.** `tools/build.sh --image`
  fails at `tools/build_phase14_pie.sh`: with the source-built sysroot the
  PIE fixture hashes to `9637c55c…a67`, not the protected
  `d7b0a8cf…ea2e`. Root cause is documented upstream
  (`docs/phase15/receipts/phase14-pie-recovery.md`): the protected identity
  requires the **official** `rust-std-x86_64-unknown-none` component, served
  only from `static.rust-lang.org`, which this sandbox cannot reach
  (allowlist: github/npm/pypi; `curl` fails with TLS error). The Phase 15
  doc's own source-built hash (`2c3720dd…`) also differs from this
  sandbox's — cause not yet isolated; recorded, not hidden. **Effect: no
  kernel rebuild, hence no new guest runs, in this sandbox.** The guard is
  not weakened. Resolution paths for M: allow `static.rust-lang.org`, or
  provide the two official component archives (manifest SHA-256 in the
  recovery doc) out of band.
- **B2 — accepted backend cannot execute here.** The pinned patched-QEMU
  binary requires `GLIBC_2.38`; the sandbox has glibc 2.36
  (`version 'GLIBC_2.38' not found`). Rebuilding it needs
  `gitlab.com` (unreachable) plus meson/ninja/glib dev packages, and a
  rebuild would be a **new backend identity** requiring re-qualification
  anyway. **Effect: strict Phase 15 `#XF` evidence cannot be regenerated
  here; unpatched-QEMU results are labeled non-qualifying wherever used.**

Neither blocker affects host-side work: the imported C runtime rebuilds
**bit-identically** to Arena 2's receipts and its host/ABI evidence
reproduces exactly against Phase 15 `main` (see `PROVENANCE.md`).
