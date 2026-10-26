# Phase 16 environment recovery log (B1/B2, 2026-10-24)

Honest record of attempts, including failures. B2 detail is in
`BACKEND-RECOVERY.md`; this file tracks B1 and the failed paths.

## B2 (patched QEMU) — RESOLVED

Failed/successful attempts, in order: (1) direct run → `GLIBC_2.38 not
found`; (2) QEMU rebuild dependency survey → meson/ninja via pip OK, no
pkg-config/glib-dev, pixman/libslirp GitHub forks stale (2016-era) —
parked; (3) glibc-from-source attempts 1–7: missing gawk/bison → GitHub
mirrors with generated configure; m4 required by bison; autotools
regeneration frozen by touching generated files; gnulib-vs-modern-glibc
patches (`SIGSTKSZ`, `__GLIBC__` guard, `_IO_*` constants); m4 man-page
failure bypassed by binary-only install; bison doc failure bypassed
likewise; **glibc 2.40 `make` exit 0, `make install` exit 0**; pinned
binary verified (controls + strict, both profiles). Host glibc untouched.

## B1 (official Rust target components) — status

Required files (from `docs/phase15/receipts/phase14-pie-recovery.md`,
channel manifest 2026-07-09):

| Component | Expected archive SHA-256 | Canonical URL (unreachable here) |
|---|---|---|
| `rust-std-1.97.0-x86_64-unknown-none.tar.xz` | `5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a` | `https://static.rust-lang.org/dist/2026-07-09/rust-std-1.97.0-x86_64-unknown-none.tar.xz` |
| `rust-std-1.97.0-x86_64-unknown-uefi.tar.xz` | `2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0` | `https://static.rust-lang.org/dist/2026-07-09/rust-std-1.97.0-x86_64-unknown-uefi.tar.xz` |

Failed attempts (exact): `curl https://static.rust-lang.org/...` →
`OpenSSL SSL_connect: SSL_ERROR_SYSCALL` (host not in sandbox allowlist:
github.com, api.github.com, codeload.github.com, registry.npmjs.org,
pypi.org only). GitHub code search for the component filenames → 0
results. PyPI `arena-rust-std`, `rust-std-x86_64-unknown-none`,
`rust-toolchain` → 404. The PyPI `arena-rust-toolchain` bundles contain
only host + wasm32 std manifests (checked `lib/rustlib/`).

Source-built identity check: repo `tools/build-sysroot.sh` output gives
PIE fixture SHA-256 `9637c55c…` here vs protected `d7b0a8cf…` (Phase 15's
own source-built attempt: `2c3720dd…`). Variant experiments with
official-style flags are recorded below as they run; the protected guard
and expected hashes are **not** modified regardless of outcome.

**Ask of M/C (if source reproduction fails):** supply the two archives
above (any transfer path), or allowlist `static.rust-lang.org`. Hashes
will be verified against the table before installation; the fixture/APB1
identities (`d7b0a8cf…`, `04add54f…`) will be reproduced and recorded.
Independent Phase 16 work continues meanwhile.

## Source-reproduction experiments (2026-10-24, concluded: not reproducible here)

Diffing the committed protected fixture (`userspace/phase14-pie/fixture.elf`)
against local builds shows: (a) build path `/workspace/ArenaOS` embedded in
a panic-location string (fixable with `--remap-path-prefix`), and (b)
different crate disambiguators (official `core` = `CsieveXhe8AWz…`, local =
`Csi1tY5tVYnWh…`) — i.e. different `core` metadata than the bootstrap-built
official libraries. Variants tried for `x86_64-unknown-none` (sysroot
restored afterwards): A) opt2+CGU1+frame-pointers+remap → fixture
`3cdfecb1…`; B) opt3+CGU1+frame-pointers+remap(+workspace path remap) →
`7f35deaf…`. Neither reproduces `d7b0a8cf…`; the disambiguator is invariant
under these flags and is driven by bootstrap-grade library metadata
(Cargo feature/cfg set), which direct `rustc` sysroot builds do not
replicate. Conclusion: the official components (table above) are required.
For development-only guest work, `tools/build_phase16_dev.sh` verifies the
committed fixture instead of rebuilding it; its artifacts are explicitly
never qualification evidence.

## Reconstruction and sandbox recovery (2026-10-25)

The later ASTR v2 / FP / live-stdin reconstruction was at one point lost
from the local worktree by an accidental local reset during sandbox
recovery; no remote history was rewritten and the pushed session branch
remained at `87987d4`. The tracked changes were reconstructed and then
rerun rather than treated as still-valid stale evidence. On a subsequent
sandbox reset, the local branch pointer was again found at `main`
(`f103926`) with the worktree edits still present and the external
toolchain absent. Before touching Git state, the worktree was archived
and verified (`/home/user/phase16-worktree-before-reattach.tar.gz`,
SHA-256 `78bdacc857da44c19b6576baea3362189ec4a3af0d9df30eb00c24491f73337c`
at recovery time); only the branch ref and index were then reattached to
fetched `87987d4`, leaving exactly the reconstructed 13 tracked
modifications and 3 untracked files visible. The environment was rebuilt
with `tools/dev-env/bootstrap.sh`, PyPI `ziglang==0.17.0`, and
`tools/dev-env/build-isolated-glibc.sh`; host and guest evidence was
regenerated under `receipts/dev-guest-2026-10-25-reconstructed/`.
B1 is unchanged by this recovery.

## B1 resolution and canonical rerun (2026-10-26)

C supplied the official archives as numbered parts in commit
`02a20fce915bdd0bdbbc24dcdc2fa0b63a1cd7a1`. Following
`docs/phase16/dependencies/rust-1.97.0/README.md`, all five parts
passed `PARTS-SHA256SUMS`; both reconstructed archives passed
`SHA256SUMS` (`5b6702d1…168a`, `2c48c8b6…1ad0`); and their official
`install.sh` scripts installed the `x86_64-unknown-none` and
`x86_64-unknown-uefi` components into private `/opt/rust/prefix`.
The reconstructed `.tar.xz` duplicates were kept outside the repo.

Protected Phase 14 then reproduced exactly only when the worktree was
bound at the physical path `/workspace/ArenaOS`; home-path and symlink
attempts failed and are retained in the canonical receipts. No build
guard or protected hash was changed. At clean `d314d20…`, canonical
host/targeted guest suites, registered 121/121, strict Phase 15, and
100/100 stability passed. The 2026-10-25 receipts remain development
evidence; canonical evidence is `receipts/canonical-2026-10-26/`.
