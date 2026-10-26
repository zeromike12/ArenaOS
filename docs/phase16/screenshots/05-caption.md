# Screenshot 05 — canonical C-native desktop before install

File: `05-c-native-desktop-canonical-2026-10-26.png`
SHA-256: `f38cb542a9b827f760d0583539c27416e85ec4e093d58af4c8284b6af7e596ba`

Genuine QMP screendump from the canonical Clang
`experiments/c-runtime/guest_phase16_canonical.py --suite native`
run at clean source commit
`d314d204aa86ec4018659f3f211cd3a488d30d87` (envelope
2026-10-26T03:24:01Z).

- EFI: `850cce0da36daa5d51452a4c4c4f01ae446b09272de293b3e14ad258f3ccaa79`
- ESP used by that suite invocation:
  `3cec1f7b2b696055e89bccb540de019c59dc89a84104408eb68d18f4d3b2641d`
- Receipts: `../receipts/canonical-2026-10-26/guest/native/`
  (`summary-clang.json`, `serial-clang-run1.log`).
- Backend: pinned patched QEMU 11.1.2 (TCG) under isolated glibc 2.40.

The seeded signed APB1 files for the no-stream negative and the C
native application are visible on the desktop before the suite's
install/launch interaction. The suite subsequently passed 9/9 groups,
98 checks, with Desktop-observed exits `[58, 57]`; that semantic
verdict is in the linked summary/serial.

Classification: **CANONICAL — QUALIFICATION CANDIDATE (not acceptance).**
