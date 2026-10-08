# ArenaOS Phase-13 complete checkpoint

`arenaos-phase13-complete-qemu-x86_64.tar.gz` is the final
100/100-receipt-bound Phase-13 archive. Its SHA-256 is recorded in the
detached `.sha256` file and `docs/phase13/FINAL-REPORT.md`.

The archive contains the exact EFI and ESP, OVMF CODE/VARS, AFS1/AFS2 scratch
image, two signed APB1 fixtures, qualification logs and receipts, tool
versions, closeout documentation, relevant ADRs, and an extracted-boot
witness. From a fresh extraction, run:

```sh
python3 phase13_archive_boot.py
```

The guest witness used QEMU 10.0.11 and only files in the extracted directory
plus that QEMU binary. Captured serial, PPM, pixel, TCP/DNS, and console output
from the final independent run are under `extracted-witness/`.
