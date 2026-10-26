# Phase 16 — Native C runtime integration: final engineering report

Date: 2026-10-26 (America/New_York)

Status: **QUALIFICATION CANDIDATE SUBMITTED FOR M/C REVIEW — NOT
ACCEPTED.** No merge to `main`, release, acceptance declaration, or
Phase 17 start is authorized by this report.

## 1. Scope and authority

Phase 16 integrates Arena 2's existing native C runtime into the
accepted Phase 15 foundation as one canonical runtime and demonstrates
signed-APB1 C applications executing inside ArenaOS. It does not claim
full POSIX or C11, C++, dynamic linking, audio, GPU acceleration, or a
major application port.

M accepted ADR-0111 (ASTR v2) and the bounded reconciled ADR-0112
(native C-runtime architecture) on 2026-10-25 as contracts only. Those
decisions did not accept this implementation or qualify Phase 16.

The canonical qualification candidate is source commit
`d314d204aa86ec4018659f3f211cd3a488d30d87` on
`arena/2b4ae1a5-arenaos`. C's dependency-only commit is
`02a20fce915bdd0bdbbc24dcdc2fa0b63a1cd7a1`; the earlier development
evidence through `bffff21f5d4a6235ec764c8596953817e157e196` remains
separately classified and was not relabelled.

## 2. What was integrated and changed

- Arena 2's runtime was selectively imported from
  `arena/f89ea987-arenaos` at `84392b8…` and kept as the one canonical
  runtime at `experiments/c-runtime/`; see `PROVENANCE.md`.
- ASTR v2 implements accepted ADR-0111 coherently in Rust and C:
  version 2, three 1,024-byte power-of-two rings in one 4 KiB region,
  wrapping `u32` occupancy arithmetic, `counter & 1023` indexing, and
  explicit refusal of v1/unknown-version/wrong-capacity pages. The old
  v1 wrap defect (`2^32 % 768 = 256`) is covered with unread data
  across the counter wrap and an explicit negative control.
- The C runtime follows bounded ADR-0112: validated ARST v2 startup,
  shared allocator, documented C subset, capability-backed streams,
  C11-subset threads/synchronization, exit/cleanup, and fail-closed
  refusals. Unsupported functions continue to return errors rather
  than fake successes.
- Live stdin is a narrow Desktop path over existing capability-owned
  streams: only the focused window's AppInstance receives keyboard
  bytes; Ctrl-D closes that instance's stdin writer after queued bytes
  drain. It is not a general POSIX terminal layer.
- The FP guest proof uses the Phase 15 ADR-0113 baseline x87/SSE2
  profile in ring 3, refuses AVX in the app audit, and leaves the
  integer-only kernel ISA audit intact.
- Notification selection is fail-closed: only a sole role-Other,
  kind-Notification, exact-RW descriptor is selected. Zero, malformed,
  or two-or-more candidates produce `NONE`; blocking stream wait then
  returns `ARENA_E_NO_WAITER` immediately.
- `guest_phase16_canonical.py` runs the unchanged targeted suites
  through canonical `mtest.build` and merges a self-identifying B1,
  source, image, backend, firmware, compiler, and verdict envelope
  into each summary. `guest_notification.py` adds the missing guest
  ambiguity proof described below.

## 3. B1 resolution and protected Phase 14 identities

The official Rust components supplied by C were installed exactly per
`docs/phase16/dependencies/rust-1.97.0/README.md`:

- every numbered part matched `PARTS-SHA256SUMS`;
- both reconstructed archives matched `SHA256SUMS`;
- each archive's official `install.sh` installed into the private
  `/opt/rust/prefix` toolchain;
- no reconstructed duplicate `.tar.xz` archive was committed;
- host system tools/glibc were not replaced.

Official archive identities:

- `x86_64-unknown-none` (14,907,620 bytes):
  `5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a`
- `x86_64-unknown-uefi` (25,374,412 bytes):
  `2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0`

Protected reproduction initially failed from `/home/user/ArenaOS` and
through a `/workspace/ArenaOS` symlink. Investigation showed the bytes
are absolute-build-path sensitive. From a physical bind mount at
`/workspace/ArenaOS`, both pristine `f103926…` and current `d314d20…`
 reproduced, with unchanged guards and scripts:

- PIE `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`
- signed APB1 `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`

The failed attempts and successful bind-mount logs are preserved in
`receipts/canonical-2026-10-26/build/`.

## 4. Canonical evidence summary

Full index and artifacts: `receipts/canonical-2026-10-26/README.md`.

Final image built from clean `d314d20…` with unchanged
`tools/build.sh --image`:

- EFI: `850cce0da36daa5d51452a4c4c4f01ae446b09272de293b3e14ad258f3ccaa79`
- ESP: `65b489016527a49e394ab6e7a57cb592045a3c80b55cceed7610aa448cb519d9`
- kernel ISA audit: PASS (120,911 instructions)

Host/static battery (overall exit 0):

- Rust `arena-runtime --lib`: 24/24.
- C runtime GCC/Clang builds and loader/ISA audits: PASS.
- ABI: 92 constants, no mismatches.
- C host: LIBC-HOST 1099 checks, 0 failures; HOST-ONLY 1707 PASS.
- Judge controls: C2 9/9, FP 12/12, Notification 6/6.

Targeted signed-APB1 guest suites, GCC and Clang (all exact PASS):

| Suite | Result |
|---|---|
| Native/C1 + ASTR checks | 9/9 groups, 98 checks, exits `[58, 57]` |
| C2 multi-app | 9/9 groups, 88 checks, xxHash exact, exits `[58, 58, 61, 63]` |
| FP x87/SSE2 | positive `66/0/0`; mutant `66/19/0` detected; exits `[58, 71, 72]` |
| Live stdin | focused attribution, burst/backpressure, EOF 118/7 bytes, no-grant + ASTR-v1 refusal; exits `[58, 73, 73, 58]` |
| Notification | production sole candidate exit 75; ambiguous `NONE`, peek `-1005`, blocking `-1012`, exit 76 |

Notification ambiguity method: an audit of windowed/headless/helper
Desktop launches, APB1 requested capabilities, stream grants, and the
service manager found no production path that creates two
role-Other/RW Notification descriptors. The guest negative therefore
uses the repository's restored-mutation pattern: production boots
first and passes; a labelled one-count Desktop fixture then adds a
distinct second descriptor; the probe must refuse selection; source
and production EFI/ESP/profile restoration is asserted byte-exact.
This proves the runtime policy in-guest without pretending a
production launcher emitted the ambiguous record.

Regressions and final-artifact checks:

- Official registered runner: **121/121 suites PASS**, clean source at
  start/end. This includes Phase 13 pressure, all Phase 14 suites,
  strict Phase 15 inside the registry, and capability-negative suites.
- Strict Phase 15 checker: PASS on both CPU profiles for the final
  Phase 16 EFI/ESP and for the preserved accepted Phase 15 package.
- Stability: **100/100 boots fully green, fail=0**, 914 s, on the exact
  final EFI/ESP. Receipt: `850cce0d…aa79 100/100`.
- Genuine canonical screenshots 04–06 are in `screenshots/`, each with
  a caption identifying its run/source/EFI/ESP and linked serial or
  stability receipt.

## 5. Demonstration package and fresh extraction

A demonstration archive was assembled from the final artifacts, the
canonical receipts, screenshots, firmware identities, a focused
runnable source subset of commit `d314d20…`, and a package manifest.
The archive identity and the post-sealing fresh-extraction guest
verification are recorded as a repository sidecar under
`receipts/canonical-2026-10-26/fresh-archive/`; the sidecar is kept
outside the sealed archive so recording its hash does not change the
artifact being verified.

Fresh-extraction verification extracts the sealed archive into a new
directory, verifies its internal checksums, confirms the extracted
EFI/ESP hashes, and boots the extracted pair with the included strict
Phase 15 checker on both CPU profiles using the pinned backend.

- Archive: `ArenaOS-Phase16-Demonstration-d314d20.tar.gz`, 4,855,009
  bytes, SHA-256
  `08786f7900581b08e65acef7466c98d56e05587e6f873c46d2c1ed0cd22225fc`.
- Attempt 1 failed before any guest boot because the long extraction
  prefix exceeded QEMU's Unix-socket path limit; the verifier/README
  were amended to fail closed on that condition, the archive was
  resealed, and the successful short-prefix attempt is the one hashed
  above.
- Successful attempt (sidecar `run.json`): static exit 0 (202 package
  files); guest exit 0, Phase 15 strict PASS on AVX-capable and
  `qemu64` profiles; extracted EFI/ESP unchanged afterwards. The
  sidecar retains both attempts' logs and both successful serials.

## 6. Issues disposition

See `ISSUES.md` for the detailed register. Summary:

- I-1 ASTR wrap corruption: **FIXED**, canonical guest verified.
- I-2 startup descriptor-count oracle mismatch: **FIXED**.
- I-3 Notification ambiguity: **FIXED/VERIFIED** in runtime host tests
  and now in guest for GCC and Clang.
- I-4 Clang weak `arena_legacy_write`: **FIXED**.
- I-5 live stdin: **FIXED**, canonical guest verified.
- I-6 process vs worker-thread exit: **VERIFIED-OK** under the
  documented last-thread-exit rule.
- I-7 graphical surface/compositor identification (B-GFX-1):
  **HANDOFF**. Not decided or implemented in Phase 16.
- I-8 pre-created console-window wake loss: **FIXED**.
- I-9 false-positive test judges/input ordering: **FIXED** without
  weakening semantic assertions.

Known remaining limitations are scope limits, not hidden failures: the
runtime is a bounded C subset; files/POSIX terminal behavior, floating
formatted output, timed/recursive mutexes, TSS, wall clock, C++,
dynamic linking, and graphical service identification remain refused
or undecided as documented in `docs/compat/C2-LIBC-COVERAGE.md` and
`ARENA1-HANDOFF.md`.

## 7. Backend and reproducibility limits

- Guest evidence used the identified locally modified QEMU 11.1.2 TCG
  backend under isolated glibc 2.40. It is not upstream-QEMU or
  hardware evidence.
- The ESP is a FAT image whose bytes vary across rebuilds; targeted
  summaries record each invocation's ESP. The final ESP named above is
  byte-fixed for strict Phase 15, stability, package, and fresh
  extraction. Any later bit change to the EFI/ESP requires
  requalification.
- Guest test bundles use the repository RFC 8032 test fixture signing
  path where summaries state it; production key management is outside
  Phase 16.
- Historical development receipts and screenshots 01–03 remain
  development evidence. Canonical claims rely only on the 2026-10-26
  receipt set.

## 8. Requested M/C review

Please review for acceptance decisions only; this report makes none:

1. Whether the canonical evidence satisfies Phase 16's bounded exit
   criteria and ADR-0111/0112 implementation expectations.
2. Whether the restored-mutation Notification guest negative is the
   correct evidence form for a state production launchers cannot emit.
3. Whether the final EFI/ESP and package identities should be accepted
   as the Phase 16 candidate artifacts.
4. Whether/when this branch may be merged. Until then: **do not merge,
   do not release, do not start Phase 17.**
