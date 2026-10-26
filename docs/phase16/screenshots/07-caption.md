# Screenshot 07 — corrected canonical C-native desktop before install

File: `07-c-native-desktop-corrected-5aacbc8.png`
SHA-256: `f38cb542a9b827f760d0583539c27416e85ec4e093d58af4c8284b6af7e596ba`

Genuine QMP screendump from the corrected canonical Clang
`experiments/c-runtime/guest_phase16_canonical.py --suite native`
run at clean source commit
`5aacbc88c50d4565a7bb9f4075be27e40df27779` (evidence envelope
2026-10-26T18:26:04.226169+00:00).

- EFI: `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
- ESP used by that suite invocation:
  `d4e2cda3073440e613eafd798d536ea5e7643b431be7dd9b500e7009c101341b`
- Receipts:
  `../receipts/canonical-corrected-5aacbc8-2026-10-26/guest/native/`
  (`summary-clang.json`, `serial-clang-run1.log`).
- Backend: pinned patched QEMU 11.1.2 (TCG) under isolated glibc 2.40.

The seeded signed APB1 files for the no-stream negative and the C
native application are visible on the desktop before the suite's
install/launch interaction. The suite subsequently passed 9/9 groups,
98 checks, with Desktop-observed exits `[58, 57]`; that semantic
verdict is in the linked summary/serial.

The PNG bytes are identical to historical screenshot 05 because the
deterministic desktop frame matches. This copy was nevertheless
regenerated and preserved from the corrected run above; its
classification comes from the linked self-identifying summary and
serial, not from the PNG alone.

Classification: **CANONICAL — CORRECTED QUALIFICATION CANDIDATE (not
acceptance).**
