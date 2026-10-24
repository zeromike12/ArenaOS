# Phase 15 qualification acceptance

**Decision: ACCEPTED by M and C on 2026-10-24** for the exact, locally modified
QEMU 11.1.2 TCG backend identified below. This records Phase 15 qualification
for the tested x87/MMX/SSE state profile and genuine CPL3 vector-19 path. It does
not qualify upstream QEMU, hardware acceleration, Windows/WHPX, every SIMD
instruction, SDL3 compatibility, or the Arena 2 C runtime.

## Reviewed evidence

- The unchanged strict guest passed 6/6 on both
  `max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`, with explicit
  `-accel tcg`, genuine vector 19, status `0x113`, containment, sibling and
  peer survival, slot/resource restoration, and clean shutdown.
- The strict T2/T3/T4 gates passed 10/10, 25/25, and 100/100. The current
  registered regression passed 121/121 suites. Expanded independent controls
  passed on the patched emulator and recorded the expected exception failures
  on the same-source unpatched emulator.
- C reviewed repository HEAD `da508c6e094b54d5f220621a8d79581ce37f06f1`
  and test/tool source `f54797f2fb5bac0b3440f986d2120382b3611ecc`; the commits
  between them changed documentation and evidence packaging only. C independently
  verified all 2,319 qualification-package hashes, reviewed all 135 strict
  stability serial logs, replayed the strict test on both CPU profiles, and ran
  the patched/unpatched controls. C inspected the existing regression and
  stability evidence; C did not rerun those long suites. Its strict replay used
  the existing TCP transport and changed no guest or test-tool source.
- For PR closeout, C reviewed PR head
  `511e69dc35a7ed51f8f75118853f797ee04a8694` and confirmed that `kernel/`,
  `userspace/`, and `tools/` match qualified source commit
  `f54797f2fb5bac0b3440f986d2120382b3611ecc`. The closeout commit is limited to
  documentation and does not alter those trees.
- The immutable qualification package SHA-256 is
  `63b36649b4498d4e6be693a619ae3c04ff1fc53fdc62008d6ef9c3cbf4c01f73`.
  C's independent acceptance-review ZIP SHA-256 is
  `095b87e686db628b95bac686241300336db6552623b8500daefb48484b73895c`.
  Both checksum sets were verified. The package was created before final
  acceptance and remains byte-for-byte unchanged so C's review stays bound to
  the exact artifact; its embedded status is the pre-review snapshot.

## Backend and scope

- Backend: locally modified QEMU 11.1.2 TCG; explicit `-accel tcg`.
- Patched emulator SHA-256:
  `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`.
- QEMU patch SHA-256:
  `c2909ea974f4ccaa9526bd5b70aef5599971bc567dd87d6ba8d9b06b327c8dd2`.
- Strict EFI SHA-256:
  `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e`.
- Matching strict ESP SHA-256:
  `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.

This is a locally modified emulator qualification. It is not an upstream-QEMU
or hardware-backed result. The emulator patch covers the documented arithmetic
and square-root helper families and tested legacy SSE/SSE2 cases; it does not
claim complete SIMD exception emulation. KVM and WHPX were not used.

## Decision boundary

M separately accepted ADR-0113 on 2026-10-24. The architecture decision and
Phase 15 qualification remain scoped to the documented x87/MMX/SSE/SSE2
profile and exact pinned locally modified TCG backend. PR
[#4](https://github.com/zeromike12/ArenaOS/pull/4) is open against `main` and
has not been merged. This record does not approve Arena 1 or Arena 2
integration, publish a release, or begin Phase 16. Arena 1 SDL3 execution and
Arena 2 C runtime/libc integration were not qualified by these tests.

See the [complete qualification record](QUALIFICATION.md),
[backend report](PATCHED-TCG-QUALIFICATION.md),
[C review report](receipts/c-acceptance-review-2026-10-24/C-REVIEW.txt),
[review bundle](receipts/c-acceptance-review-2026-10-24/review-bundle.zip), and
[downloadable qualification package](artifacts/phase15-patched-tcg-qualification.tar.gz).
