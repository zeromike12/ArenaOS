# Screenshot 09 — corrected final-artifact stability boot 100

File: `09-stability-boot-100-corrected-5aacbc8.png`
SHA-256: `4d2e29d77bec530ca5c1ef0cef8d3813655c4cf53672724488c05d328ee82ede`

Lossless PNG conversion of the genuine QMP PPM retained by
`tools/stability_loop.sh 100` for boot 100. The source PPM SHA-256 is
`223f56b95d261141385b40d07c5d9d86e70186ce003aed4e59951e066b5d0fe0`,
byte-identical to the corrected run's boot-1 PPM.

- Source commit: `5aacbc88c50d4565a7bb9f4075be27e40df27779`
- Final EFI:
  `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
- Final ESP:
  `04ffaf4681e92fe87886bdcff29b175df032ae9d88bd6cae59b34ad070ea674a`
- Stability receipts:
  `../receipts/canonical-corrected-5aacbc8-2026-10-26/stability/`
  (`stability-100.log`, `stability-receipt.txt`, `run.json`, boot-100
  serial, and hashes/text for all 100 pixel receipts).
- Backend: pinned patched QEMU 11.1.2 (TCG) under isolated glibc 2.40;
  preserved Phase 15 package OVMF firmware.

The corrected loop completed **100/100 boots fully green, fail=0**, in
942 s. This is the desktop frame from the final boot; per-boot semantic
checks were performed by the stability script and are not inferred
from this image. The prior candidate's stability receipt was not
reused.

Classification: **CANONICAL — CORRECTED QUALIFICATION CANDIDATE (not
acceptance).**
