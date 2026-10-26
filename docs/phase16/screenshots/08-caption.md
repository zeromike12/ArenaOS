# Screenshot 08 — corrected canonical live-stdin run

File: `08-live-stdin-corrected-5aacbc8.png`
SHA-256: `8507d3ab1fb4a88b25292f2aa3cd1523851dac04c96d6e9f164534f11af3e5fc`

Genuine QMP screendump written by `experiments/c-runtime/guest_stdin.py`
during the corrected canonical Clang run at clean source commit
`5aacbc88c50d4565a7bb9f4075be27e40df27779` (evidence envelope
2026-10-26T18:30:21.920082+00:00).

- EFI: `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
- ESP used by that suite invocation:
  `b6578d45be1c76dab467ff2a197b392194954f1eb709d0e885ebb6dc54f3c6f9`
- Receipts:
  `../receipts/canonical-corrected-5aacbc8-2026-10-26/guest/stdin/`
  (`summary-clang.json`, `serial-clang-run1.log`).
- Backend: pinned patched QEMU binary
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`
  under the isolated glibc 2.40 wrapper; host glibc unchanged.

The two signed C input-probe instances are visible. The semantic
focused-key attribution, burst/backpressure, no-grant refusal,
ASTR-v1 refusal, and Ctrl-D EOF evidence (118 and 7 bytes) is in the
linked serial receipt, not inferred from pixels.

Classification: **CANONICAL — CORRECTED QUALIFICATION CANDIDATE (not
acceptance).**
