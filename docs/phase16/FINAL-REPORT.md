# Phase 16 — Native C runtime integration: final engineering report

Date: 2026-10-26 (America/New_York)

Status: **CORRECTED QUALIFICATION CANDIDATE SUBMITTED FOR M/C REVIEW —
NOT ACCEPTED.** No merge to `main`, release, acceptance declaration,
or Phase 17 start is authorized by this report.

## 1. Correction closeout and candidate identity

After the earlier `d314d20…` qualification candidate was assembled,
review found that ADR-0111's “all reserved bytes are zero” rule was
not enforced completely or consistently by the Rust and C ASTR v2
validators. The bounded correction below strengthens validation; it
does not weaken ADR-0111 or change the wire layout.

The current corrected canonical candidate is OS/test source commit
`5aacbc88c50d4565a7bb9f4075be27e40df27779` on
`arena/2b4ae1a5-arenaos`. Its receipts are
`receipts/canonical-corrected-5aacbc8-2026-10-26/`.

Correction chain, committed before final qualification:

1. `8c472a2fe110b6bb3db81145342043de5ee75e03` — production and
   host-test ASTR v2 reserved-byte enforcement in Rust and C.
2. `d5874b72f857261fe3c41eb7e7ac7d8af6431463` — harness-only repair
   to the test-generated legacy ASTR-v1 fixture transform.
3. `5aacbc88c50d4565a7bb9f4075be27e40df27779` — formatting-only
   `rustfmt` correction to two assertions in the new Rust tests.

The earlier `d314d20…` receipts under
`receipts/canonical-2026-10-26/` remain unchanged historical candidate
evidence. They have not been overwritten or relabelled, and their
image/stability receipts were not reused for the corrected image.

Corrected final artifacts:

- EFI (11,038,720 bytes):
  `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
- ESP (16,777,216 bytes):
  `04ffaf4681e92fe87886bdcff29b175df032ae9d88bd6cae59b34ad070ea674a`
- Protected Phase 14 PIE:
  `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`
- Protected signed Phase 14 APB1:
  `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`

## 2. Scope and authority

Phase 16 integrates Arena 2's existing native C runtime into the
accepted Phase 15 foundation as one canonical runtime and demonstrates
signed-APB1 C applications executing inside ArenaOS. It does not claim
full POSIX or C11, C++, dynamic linking, audio, GPU acceleration, or a
major application port.

M accepted ADR-0111 (ASTR v2) and the bounded reconciled ADR-0112
(native C-runtime architecture) on 2026-10-25 as contracts only. Those
decisions did not accept this implementation or qualify Phase 16.

C's dependency-only commit is
`02a20fce915bdd0bdbbc24dcdc2fa0b63a1cd7a1`; earlier development
evidence remains separately classified and was not relabelled.

## 3. What was integrated

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
  into each summary. `guest_notification.py` adds the guest ambiguity
  proof described below.

## 4. Reserved-byte correction

ASTR v2 geometry is unchanged: one 4096-byte page; header reserved
16–63; rings at 64, 1152, and 2240; each ring control reserved 12–63;
reserved page tail 3328–4095; capacity 1024; version 2.

### Rust

In `userspace/arena-runtime/src/streams.rs`:

- `region_ptr` now requires exactly `STREAM_PAGE_BYTES` (4096) before
  any caller memory is dereferenced. Sub-page and overlong lengths are
  refused; this establishes that the supplied range makes the full
  page readable before the tail is inspected and avoids out-of-range
  reads.
- `attach` checks the reserved tail after that exact-length gate and
  combines it with the existing header/ring reserved checks.
- `initialize` explicitly zeroes the reserved tail.
- Three focused tests cover both endpoints of every reserved range, a
  valid/restored positive control, length boundaries 0/3327/3328/4095,
  and full-page initializer zeroing from nonzero backing bytes.

### C

In `experiments/c-runtime/src/streams.c` and
`experiments/c-runtime/include/arena/rt.h`:

- `arena_ring_page_check` checks header reserved bytes, each ring's
  control bytes 12–63, and tail bytes 3328–4095.
- Static assertions pin the v2 tail to offset 3328 and the complete
  layout to one page.
- The existing bare-pointer API contract is now documented precisely:
  `page` must name one complete 4096-byte, 4096-aligned readable page
  (writable for mutating calls). The API takes no length and cannot
  safely detect a shorter object in C.
- Host tests probe both endpoints of the header, all three ring
  controls, and the tail on a complete page, with valid/restored
  positive controls.

### Sensitivity

The new tests detect the prior omissions:

- Previous C production logic fails exactly 8/1721 host checks, at
  page offsets 76, 127, 1164, 1215, 2252, 2303, 3328, and 4095.
- Previous Rust production logic passes the 24 pre-existing tests and
  fails exactly the three new tests.
- Corrected code passes Rust 27/27 and C host 1721 checks.

## 5. B1 resolution and protected Phase 14 identities

The official Rust components supplied by C were installed exactly per
`docs/phase16/dependencies/rust-1.97.0/README.md`:

- every numbered part matched `PARTS-SHA256SUMS`;
- both reconstructed archives matched `SHA256SUMS`;
- each archive's official `install.sh` installed into the private
  `/opt/rust/prefix` toolchain;
- no reconstructed duplicate `.tar.xz` archive was committed;
- host system tools/glibc were not replaced.

Official archive identities:

- `x86_64-unknown-none`:
  `5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a`
- `x86_64-unknown-uefi`:
  `2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0`

Installed `libcore` identities:

- none:
  `6b75c5188dca46c917a67ade52dddea81460f96dc4732003cebea2e22a5b1b09`
- UEFI:
  `a7de36b0262b7d94cdcf1acb3f78288e7f5a57d546568b3ccaec27cd122c5494`

Protected reproduction is absolute-build-path sensitive. From the
physical bind mount `/workspace/ArenaOS`, unchanged guards reproduced
both Phase 14 identities above. No guard or expected hash was changed.

## 6. Corrected canonical evidence

Full index:
`receipts/canonical-corrected-5aacbc8-2026-10-26/README.md`.

Host/static battery (overall exit 0):

- Rust `arena-runtime --lib`: 27/27.
- C runtime GCC/Clang builds and loader/ISA audits: PASS.
- ABI: 92 constants, no mismatches.
- C host: LIBC-HOST 1099 checks, 0 failures; HOST-ONLY 1721 PASS.
- Judge controls: C2 9/9, FP 12/12, Notification 6/6.

Targeted signed-APB1 guest suites, GCC and Clang (all exact PASS):

| Suite | Result |
|---|---|
| Native/C1 + ASTR checks | 9/9 groups, 98 checks, exits `[58, 57]` |
| C2 multi-app | 9/9 groups, 88 checks, xxHash exact, exits `[58, 58, 61, 63]` |
| FP x87/SSE2 | positive `66/0/0`; mutant `66/19/0` detected; exits `[58, 71, 72]` |
| Live stdin | focused attribution, burst/backpressure, EOF 118/7 bytes, no-grant + ASTR-v1 refusal; exits `[58, 73, 73, 58]` |
| Notification | production sole candidate exit 75; ambiguous `NONE`, peek `-1005`, blocking `-1012`, exit 76 |

Notification ambiguity method: an audit of production launch paths
found no way to create two role-Other/RW Notification descriptors.
The guest negative uses the repository's restored-mutation pattern:
production boots first and passes; a labelled one-count Desktop
fixture then adds a distinct second descriptor; the probe must refuse
selection; source and production EFI/ESP/profile restoration is
asserted byte-exact. C accepted this evidence form; it is not a claim
that a production launcher emitted the ambiguous record.

### Attempts and retries

The correction history includes failed attempts that remain failures:

- At `8c472a2…`, both live-stdin targeted invocations failed before
  boot in the generated legacy-v1 fixture. Commit `d5874b7…` fixed
  only that harness transform.
- At `d5874b7…`, the complete registered run failed 1/121 after
  8181 s solely on `cargo fmt --check` for two new Rust assertions.
  The original full external log was lost in a sandbox reset; only a
  focused exact reproduction is claimed. Commit `5aacbc8…` is the
  formatting-only correction.
- At final source `5aacbc8…`, registered attempt 1 failed 2/121 after
  6543 s (`test_m84_crash.py`, `test_m9_client_death.py`). Focused
  reruns passed, but were not substituted for the complete suite.
- Registered attempt 2 at the same clean source passed all 121 suites
  in 6574 s. No root cause for attempt 1's two failures is claimed;
  their non-reproduction in focused and complete reruns is recorded
  for M/C review.

Final-artifact checks on the corrected pair:

- Strict Phase 15 checker: PASS on both CPU profiles for the corrected
  final pair and the preserved accepted Phase 15 package.
- Stability: **100/100 boots fully green, fail=0**, 942 s, on the
  exact corrected EFI/ESP. Receipt: `7dcee442…2bfe6 100/100`.
- Genuine corrected screenshots 07–09 are in `screenshots/`, each with
  a caption identifying run/source/EFI/ESP and linked receipts.

## 7. Demonstration package and fresh extraction

The corrected demonstration archive is
`artifacts/ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz`, with its
external SHA-256 sidecar beside it in `artifacts/`. It contains the
corrected final artifacts and protected Phase 14 artifacts, firmware,
corrected receipts, screenshots 07–09, submission/ADR/compat docs, a
focused tracked-file source subset of tested commit `5aacbc8…`, a
manifest/checksums, and `verify_package.py`.

The whole-archive SHA-256 and post-sealing fresh-extraction guest
receipts are recorded outside the sealed archive under
`receipts/canonical-corrected-5aacbc8-2026-10-26/fresh-archive/`, so
recording the archive identity cannot change the artifact being
verified. Fresh verification uses a short extraction prefix, checks
all internal digests and manifest identities, confirms the ESP's inner
EFI byte-for-byte, and boots the extracted pair with the included
strict Phase 15 checker on both CPU profiles using the pinned backend.

- Archive: `ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz`, 5,016,910
  bytes, SHA-256
  `106088522042fb501a00554442ae97133127e53a9141d5e95069f112c9a9898b`.
- Repository download:
  <https://github.com/zeromike12/ArenaOS/blob/arena/2b4ae1a5-arenaos/docs/phase16/artifacts/ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz>
- Fresh run: sidecar check exit 0; static exit 0 (225 package files
  plus `SHA256SUMS`, 100-file source subset, ESP inner EFI exact);
  guest exit 0 in 15 s with strict Phase 15 PASS on both CPU profiles;
  extracted EFI/ESP and archive hash unchanged afterwards. Raw logs
  and serials are in the `fresh-archive/` sidecar.

## 8. Issues disposition

See `ISSUES.md` for the detailed register. Summary:

- I-1 ASTR wrap corruption: **FIXED**, corrected canonical verified.
- I-2 startup descriptor-count oracle mismatch: **FIXED**.
- I-3 Notification ambiguity: **FIXED/VERIFIED** host + guest.
- I-4 Clang weak `arena_legacy_write`: **FIXED**.
- I-5 live stdin: **FIXED**, corrected canonical verified.
- I-6 process vs worker-thread exit: **VERIFIED-OK** under the
  documented last-thread-exit rule.
- I-7 graphical surface/compositor identification (B-GFX-1):
  **HANDOFF**. Not decided or implemented in Phase 16.
- I-8 pre-created console-window wake loss: **FIXED**.
- I-9 false-positive test judges/input ordering: **FIXED** without
  weakening semantic assertions.
- I-10 ASTR v2 reserved-byte enforcement gaps: **FIXED**, with
  negative, boundary, positive-control, sensitivity, host, and guest
  evidence at `5aacbc8…`.
- Q-1 registered attempt-1 failures at `5aacbc8…`: **RETAINED / NOT
  REPRODUCED** in focused reruns or complete attempt 2; no root cause
  or product fix is claimed because no source changed between those
  registered attempts.

Known remaining limitations are scope limits, not hidden failures: the
runtime is a bounded C subset; files/POSIX terminal behavior, floating
formatted output, timed/recursive mutexes, TSS, wall clock, C++,
dynamic linking, and graphical service identification remain refused
or undecided as documented in `docs/compat/C2-LIBC-COVERAGE.md` and
`ARENA1-HANDOFF.md`.

## 9. Backend and reproducibility limits

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
- Historical development receipts/screenshots and the `d314d20…`
  canonical-candidate receipts remain separately labelled. Corrected
  claims rely on the corrected `5aacbc8…` receipt set.
- Registered attempt 1's two non-reproduced failures are retained as a
  qualification-history caveat rather than silently discarded.

## 10. Requested M/C review

Please review for acceptance decisions only; this report makes none:

1. Whether the reserved-byte correction and sensitivity evidence fully
   implement ADR-0111 without altering its accepted contract.
2. Whether the corrected canonical evidence, including the retained
   failed attempts and complete registered attempt 2, satisfies Phase
   16's bounded exit criteria.
3. Whether the restored-mutation Notification guest negative remains
   the correct evidence form for a state production launchers cannot
   emit.
4. Whether the corrected final EFI/ESP and sealed package identities
   should be accepted as the Phase 16 candidate artifacts.
5. Whether/when this branch may be merged. Until then: **do not merge,
   do not release, do not start Phase 17.**
