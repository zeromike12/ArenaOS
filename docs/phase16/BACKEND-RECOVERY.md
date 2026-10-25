# Phase 16 backend recovery (B2 resolved, 2026-10-24)

## Identity

| Layer | Identity |
|---|---|
| Emulator binary | **the exact pinned Phase 15 binary**, SHA-256 `e0ea2b1f…f26ff` (extracted from `docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz`); `--version` → `QEMU emulator version 11.1.2`; `-accel help` → `tcg` only |
| Runtime libraries (package) | GLib/Pixman/Slirp objects from the same package (`backend/runtime/lib`) |
| Host handicap | sandbox glibc 2.36 < required 2.38 → binary would not start (`version 'GLIBC_2.38' not found`) |
| Isolated runtime | glibc **2.40** built from source into `/home/user/glibc-2.40` (host system glibc untouched). Source: `github.com/bminor/glibc` branch `release/2.40/master` @ `9fe8576664d43b87ca19401fb6a975e217e47623`. Built `libc.so.6` SHA-256 `3c26ae81…772a`, `ld-linux-x86-64.so.2` `24e9a58c…ddb2`. Invocation: explicit new `ld.so --library-path /home/user/glibc-2.40/lib:<pkg>/backend/runtime/lib:/usr/lib/x86_64-linux-gnu <pinned binary> -accel tcg -L <pkg>/backend/pc-bios` |
| Firmware / images | package `OVMF_CODE_4M.fd`/`OVMF_VARS_4M.fd`; strict EFI `bbbc63df…238e`, ESP `631cc645…866f` (all match `docs/phase15/ACCEPTANCE.md`) |

The emulator binary is bit-identical to the accepted one; the *runtime
environment* is new, so per assignment the combination was re-verified
rather than inheriting qualification. Verified scope below is what was
actually run — nothing more is claimed.

## Build-tool provenance (glibc chain; tools only, nothing shipped)

apt/Debian mirrors are unreachable in this sandbox. Private build tools
were built from GitHub mirrors with generated `configure` scripts:
gawk 4.0.70 (`gvlx/gawk` @ `a8922935…`), GNU m4 1.4.18 (`tar-mirror/gnu-m4`
@ `f1a9aff6…`, small gnulib-compat patches for modern glibc headers:
`SIGSTKSZ` guard, `_IO_*` flag defines), bison (`Distrotech/bison` @
`48b6e4b0…`; only `src/bison` + `data/` installed — its full `make` exits 2
on documentation, recorded). Failed attempts are preserved in the session
log summary in `RECOVERY-LOG.md`. None of these enter any ArenaOS artifact.

The QEMU-rebuild alternative (GitHub `qemu/qemu` tag `v11.1.2`, peeled
`4fc49f46…` — confirmed present) was investigated and **not** needed:
meson 1.12.1/ninja (pip) exist, but pixman/libslirp have no suitable
GitHub-official sources (only stale forks), and a rebuild would be a new
backend identity requiring full requalification regardless. Running the
exact binary was strictly better.

## Verification actually executed

1. **Explicit TCG init**: `-accel help` lists only `tcg`; all runs pass
   `-accel tcg` explicitly (wrapper).
2. **Independent emulator controls** (rebuilt this session:
   EFI SHA-256 `8d06a67b…af`, ESP `9bb8b79c…ad`):
   - patched: `RESULT PASS: expanded independent TCG #XM controls` (16/16)
   - unpatched (same package, SHA-256 `443c81fc…8484`): `RESULT FAIL`
     with the expected #XM failures (sticky-IE, OSXMMEXCPT, ZE, SQRTPS)
   Receipts: `receipts/backend-controls-{patched,unpatched}-serial.txt`.
   (The bare UEFI control app does not self-shutdown; runs were timed out
   at 120 s *after* complete RESULT lines — recorded, not hidden.)
3. **Unchanged strict Phase 15 guest, both CPU profiles**
   (`tools/test_m15_guest.py`, package EFI/ESP/firmware):
   **PASS** — `max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`, genuine
   vector 19, clean shutdown, exit 0. Receipts:
   `receipts/backend-strict-run.txt`,
   `receipts/backend-strict-serial-{max,qemu64}.txt`.

## Status vs B1

Guest *execution* is recovered. Guest *integration* (new C apps in a
rebuilt kernel) still needs B1: official Rust target components for the
protected Phase 14 fixture identity. See `RECOVERY-LOG.md`.
