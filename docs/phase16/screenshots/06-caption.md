# Screenshot 06 — final-artifact stability boot 100

File: `06-stability-boot-100-canonical-2026-10-26.png`
SHA-256: `b31e5288e36b1b534c91e8cbec3b2111e27ec82b381a56dd8d87804c79a15837`

PNG conversion of the genuine QMP PPM retained by
`tools/stability_loop.sh 100` for boot 100
(`build/stability-display-boot-100.ppm`, SHA-256
`223f56b95d261141385b40d07c5d9d86e70186ce003aed4e59951e066b5d0fe0`).

- Source commit: `d314d204aa86ec4018659f3f211cd3a488d30d87`
- Final EFI:
  `850cce0da36daa5d51452a4c4c4f01ae446b09272de293b3e14ad258f3ccaa79`
- Final ESP:
  `65b489016527a49e394ab6e7a57cb592045a3c80b55cceed7610aa448cb519d9`
- Stability receipts:
  `../receipts/canonical-2026-10-26/stability/`
  (`stability-100.log`, `stability-receipt.txt`, `run.json`,
  boot-100 serial, and hashes for all 100 pixel receipts).
- Backend: pinned patched QEMU 11.1.2 (TCG) under isolated glibc 2.40;
  package OVMF firmware.

The loop completed **100/100 boots fully green, fail=0**, in 914 s.
This is the desktop frame from the final boot; the per-boot semantic
checks were performed by the stability script and are not inferred
from this image.

Classification: **CANONICAL — QUALIFICATION CANDIDATE (not acceptance).**
