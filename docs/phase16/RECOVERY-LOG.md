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
