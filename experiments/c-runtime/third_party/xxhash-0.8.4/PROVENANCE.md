# Vendored upstream: xxHash v0.8.4 (UNMODIFIED)

- Project: https://github.com/Cyan4973/xxHash
- Release tag: v0.8.4, tag commit `c87183a77d67f7d37e3d2d1b7eaac5e7c695e4f0`
- Source archive: https://codeload.github.com/Cyan4973/xxHash/tar.gz/refs/tags/v0.8.4
- Archive SHA-256: `5738270935e7c3d38a79b3adf7c9692566ce7895a25f67de43ad52ab504acd32`
- License: BSD 2-Clause (see `LICENSE`, copied verbatim from the upstream archive)
- Files vendored, byte-identical to the archive: `xxhash.h`, `xxhash.c`, `LICENSE`

The files are NOT modified. The library is compiled with the ArenaOS C2 runtime
flags only (no SSE/MMX/x87: the scalar path is selected because `__SSE2__` is
not defined under `-mno-sse`). Its only libc dependencies are `malloc`/`free`
(state objects), `memcpy`/`memset`, and `<limits.h>`/`<stddef.h>` constants.
No file, graphics, floating-point, or dynamic-loading facility is used.

Integrity: see `SHA256SUMS` in this directory (checked by `run.py audit`).
