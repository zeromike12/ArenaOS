# Screenshot 03 — live-stdin development run

File: `03-live-stdin-dev-2026-10-25.png`
SHA-256: `7b1b33e6acddd80850a17eb4b9ae500348be047893b83eac5449bfc27c1f0656`

Genuine QEMU screendump from `experiments/c-runtime/guest_stdin.py` on the
2026-10-25 reconstructed **development** image:

- EFI: `cd32caf4816247ad99862bd548d0248c5e7d409e8f119ef93fc84d56809219c9`
- ESP: `0524e6c3fb51b8438a3657b41fd2bd5e2505966b3bfbefcbc03c41e0d4144513`
- Source at run time: pushed `87987d4` plus the dirty paths recorded in
  `../receipts/dev-guest-2026-10-25-reconstructed/stdin/summary-clang.json`;
  subsequently committed through `a7ad4c2`.
- Backend: pinned patched QEMU 11.1.2 through the isolated glibc 2.40
  wrapper; exact hashes are in the receipt README.

The screenshot shows the two simultaneously running C input-probe
instances (Selma behind, Marina focused in front), each in its own
pre-created console window. The guest C apps report their byte-level
echo/EOF evidence on their granted stdout streams, not by drawing into the
window surface; see the stdin summaries and raw serials in
`../receipts/dev-guest-2026-10-25-reconstructed/stdin/`.

Classification: **DEVELOPMENT ONLY — NOT QUALIFICATION EVIDENCE** (B1).
