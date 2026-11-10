# Supplements to the strict r4 evidence archive

The strict archive and its embedded manifest remain unchanged. Additional raw materials are published separately:

- `../v1-acceptance-7a187b0-r4-stability-raw.tar.gz` — per-boot pixel/DNS receipts and the first/last genuine QEMU stability screenshots.
- `../v1-acceptance-7a187b0-r4-raw-ppm.tar.gz` — all 84 original PPM captures excluded from the strict PNG package.
- `../v1-phase13-7a187b0-r2.tar.gz` — full standalone Phase 13 regression evidence with its 600-second main timeout unchanged.

The full gate stdout and shell exit code are included under `gate/` for convenient review. Exit 3 is the aggregate's BLOCKED result due to Requirement 4b OPEN; it is not a stage-test failure and is not an acceptance PASS.
