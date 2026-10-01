# Phase 8.5 implementation checkpoint — NOT qualification

This source checkpoint follows the qualified Phase 8.4 image. ADR-0054/0055
remain the accepted architecture; **Phase 8.5 is incomplete**. It must not
replace the Phase 8.4 deployable/checksummed release in `RUNNING.md`.

## Implemented and exercised in real guests

- The existing `packaged` endpoint accepts the distinct manager-only
  lifecycle marker for INSTALL, SELECT_PREPARE, SELECT_COMMIT, ABORT,
  DEACTIVATE and explicit LAUNCH. AINS/AACT exact 512-byte immutable records
  are committed through fsd and reread; a corrupt visible newest record
  refuses READY rather than falling back. INSTALL repeats without another
  CREATE. PREPARE has a single-use in-memory token, duplicate PREPARE and
  ABORT are tested; COMMIT returns a provisional Image only after the
  selected record is durable. A separate explicit LAUNCH re-registers and
  transfers a fresh Image cap. The manager checks Image kind/full ID/rights,
  attenuates its LAUNCH cap, and starts/reaps one real ring-3 child through
  its held Process cap; a second unretired dynamic child is refused BUSY.
- Two different purpose-built, production-ELF-validator-checked payloads
  (648 and 656 bytes), independently signed **offline with the published test
  root**, execute as versions 7 and 8 on separate boots. AINS2 links AINS1;
  AACT4 selects version 8 after the committed version-7 decisions. The
  second-version test does **not** exercise a live old/new child overlap.
- The v1 POLICY request now refuses with its existing `DENY` status before
  CREATE whenever a durable selection is active: the older staging marker
  has no authority to stop the child or revoke Image copies. A guest upgrade
  fixture presents a valid independently root-signed next policy that would
  revoke the selected v7 package, observes that refusal, and verifies no
  new policy record was written. This is a conservative mutation freeze,
  **not yet proof of a live revocation/deactivation cutover**.
- `test_m85_select.py` proves three committed decisions and a fourth
  pre-CREATE `NO_SPACE` on a **synthetic** full 32-object platter, with
  unchanged three prior records. This is not a maximal historical fixture.
  `test_m85_crash.py` SIGKILLs at INSTALL and first AACT CREATE/WRITE/CLOSE
  boundaries, classifies only AFS1's legal ordered-commit prefixes and
  reboots each platter. No hostile rollback or arbitrary commit-sector
  corruption claim follows.
- The manager-death negative control expects the **fatal** kernel halt when
  the manager exits with a live provisional ID. QEMU's firmware exit code
  zero is mapped by the harness to semantic refusal `97`, not a success.
  The production `Image` ref/pin conservation walker caught an intentionally
  omitted IPC reply-cap credit (RED) and the exact restored source/EFI
  passed the real guest again (GREEN). This covers one production hook,
  **not** all IPC/death/rollback paths.
- The preceding pushed implementation checkpoint (`c1766d1`) completed
  `tools/run_tests.sh` with **ALL TESTS PASSED (54 test suites)**. Its build
  produced an EFI and ESP; the intermediate SHA-256 was
  `aca3268824f4aefda1ddf2642ad3798ad3a0d3d255805c1ef8afc5800a871d86`
  (EFI) and `a6e224c2c5de6ad6ea1ccb794909228078232750028848534a0da32c2d16bbb6`
  (ESP). The later active-selection POLICY freeze passed the targeted
  two-image QEMU upgrade test on the recovered workspace; rerun the full
  suite here after the final code change. These are not final artifact-bound
  qualification receipts.

## Reproduce or boot the source checkpoint

On a machine with the pinned Rust bare-metal targets, QEMU and OVMF (in the
Arena sandbox the repository's `tools/dev-env/bootstrap.sh` provisions them):

```sh
source tools/dev-env/env.sh
tools/build.sh --image       # build/arena-boot.efi + build/arena-esp.img
tools/run_tests.sh           # historical and new guest tests
tools/run.sh                 # interactive QEMU; type shutdown to exit
```

Use an empty scratch disk for the initial historical M5 boot; the 8.5 guest
scripts seed only **public signed test artifacts** after that boot. Never
seed private keys or interpret the unseeded shell as an installer UI. The
fixed `app.test` `installtest`, `selecttest`, `selectlite` and `upgradetest`
commands are **test-only Power-shell requests**, not a generic package
picker or production update workflow.

## Not yet closed

Concurrent old/new registered Image lifetime, a one-child-bounded live
cutover, policy-revocation/deactivation cutover and already-running child
teardown; complete Image reference and all manager-
death paths; observed guest resource high-water marks; actual maximal
historical AFS1 fixture; robust negative ABI and token tests; and production
custody remain outstanding. The currently accepted test root is not a
production key. After the **last** code change the full static/historical
suite, fresh exact-final-EFI-bound 100/100 from zero, checksum-verified
independently extracted/booted deployable bundle and final evidence are
still mandatory. Do not mark Phase 8.5 complete from this checkpoint.
