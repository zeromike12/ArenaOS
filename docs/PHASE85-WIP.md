# Phase 8.5 rebuilt qualification — evidence ledger

ADR-0054/0055 are the accepted architecture. This ledger refers to the **current rebuilt tree**, not observations from the missing pre-`6a96911` work. Source-preservation commits are not milestone qualification. The rebuilt source passed its historical suite and exact-final-EFI 100/100; the checksummed archive was independently extracted and booted. The final source/artifact commit was pushed to this branch.

## Actual guest proof in this rebuild

- `tools/test_m85_live_cutover.py`/`test_m85_resources.py`: distinct production-validator-accepted ELF payloads are signed **offline by the public test root** as v7 and v8, staged, installed, selected and actually run in ring 3. The signed old v7 child remains **alive** while PREPARE registers v8 in the second kernel Image slot. Third registration is BUSY, as is a second dynamic spawn even when the previous tagged child is exited but unretired. The manager holds the Process cap, STOP/FINISH retires the child, revokes all copies of the old Image ID **before** durable AACT4, and runs the distinct v8 child. Four further old-child STOP/FINISH/BUSY cycles have equal before/after cap counts.
- Guest-observed resource samples (free frames, records, processes): `(116628,12,12) → (116614,13,13) → (116628,12,12)`. Manager cap occupancy: baseline `25`, live child `27`, **two simultaneous live Image IDs** `28`, after FINISH `26`, cycle before/after `26/26`; packaged initial `5`, observed peak `7`. Existing manager Notification reached **18/18**, nineteenth creation was refused without mutation. These are fixture measurements, not a general production memory guarantee.
- `tools/test_m85_maximal.py`: an authentic historical platter reaches exactly **32/32** AFS1 slots: 19 existing objects, eight actual historical state records, two real guest-created AINS records and three guest-created AACT records. A fourth conditional AACT decision is refused `NO_SPACE` **before CREATE**, the whole disk is byte-exact before/after, and a previously signed v7 Image still relaunches. No bound increase, compaction, GC or invented ninth v1 policy slot.
- `tools/test_m85_crash.py` and `test_m85_crash_upgrade.py`: genuine guest CREATE/WRITE/CLOSE SIGKILL/reboot prefixes for AINS1/AACT1 and AINS2/AACT4 respectively, audited as absent, visible empty or exact complete record against the documented AFS1 ordered-write/atomic-sector model. This does **not** claim resilience to arbitrary commit-sector corruption, hostile rollback or Secure Boot.
- `tools/test_m85_ref_hook.py`: omit the production IPC reply-cap credit in a temporary mutant: the **independent kernel ref/pin conservation walker** halts on the real guest signed-cap path (RED); exact source/EFI/ESP restoration and guest rerun pass (GREEN). `test_m85_manager_death.py`, `test_m85_manager_death_live.py`, `test_m85_manager_destroy.py`: normal and ring-3-fault manager exit with a provisional ID or genuinely live signed child fail-stop; the separate direct test-only caller routes into the **unchanged production `proc::destroy` guard** and observes the same fatal before teardown. The destroy diagnostic insertion and mutant images are restored byte-exactly.
- `test_m85_install.py`, `test_m85_select.py`, `test_m85_upgrade.py`, Phase-8.5 host codec/model/capacity proofs and earlier Phase-8.4 real guest negatives remain in the historical suite. All caps and records are exercised by the real guest; host scripts only feed signed bytes prepared offline and inspect independent disk/hash/serial observations.

## Diagnosed regression during the historical gate

The first full-suite run exposed a real failure, **not a flake**: the 32/32
fixture stopped a child whose IPC call had just woken a server, but process
teardown discarded the child's `Delivered` queue slot before the server
resumed. The server's `SYS_IPC_RECV` then found no delivery and halted.
The internal queue now preserves a one-shot caller-cancellation tombstone
(with no retained cap or caller), so the woken server consumes the event and
re-enters its receive loop. If the server already returned from recv, its
next recv retires the tombstone before parking; server death also clears
unused tombstones.
The endpoint and call-slot layouts did not change. The original failing
serial is `build/serial-m85-maximal-maximal.log` only until a new test
rewrites that path; the permanent failure record is
`build/phase85-pre-ipc-cancel-failed-suite.log` from the interrupted run, where the
`test_m85_maximal.py FAILED` trace remains. The corrected target-only
32/32 guest, the earlier three-boundary 8.2 IPC dead-caller guest and the
signed live-cutover resource guest pass. The full suite was rerun from zero after this change: **60/60 PASS**.
The first attempted final-image stability run scored six failures before it
was stopped: its shell gate still required the old manager `20` boot caps
and `17/17` Notification marker, although each captured guest boot showed
`22` caps and `18/18` and a clean shutdown. A second interrupted run then failed its obsolete packaged grant string
(`marker/R` instead of the receiver's actual `STAGE/R registrar/W
lifecycle/R`). The gate and extracted-bundle gate now check the correct
signed 8.5 receiver inventory. Neither interrupted run is counted toward
100/100; the full final-EFI-bound run restarts at boot one. Both were stale
harness expectations, **not** the earlier unexplained pre-kernel OVMF
stall.

## Boundaries

The Power `pkg ...test`, `selectlite`, `select`, `oldlive`, `upgradetest`, `installtwo` and `maximalselect` commands are **private test-fixture requests**, not a production package picker. The public test root is not a production signing custody procedure. The manager and packaged registrar bearers are both in the dynamic-execution TCB. Authority is by possessed, receiver-checked rights/markers and held Process/Image capabilities, never by process name. The single unretired dynamic-child bound is system-wide. A POLICY that would revoke an active selected package is conservatively refused before CREATE; no general live policy rotation claim is made. Future key custody/rotation, hostile rollback, arbitrary disk corruption, anti-rollback, dynamic linking, GC and desktop work require separate decisions.

## Qualification and reproduction

After the last kernel change, `source tools/dev-env/env.sh; bash tools/run_tests.sh` completed **ALL TESTS PASSED (60 test suites)** (`build/phase85-qualified-suite.log`). `bash tools/build.sh --image` produced EFI SHA-256 `e534f7bada1ce1f61a300e659101415a1ed79874108a7021c1e02eea91d034d5`; `bash tools/stability_loop.sh 100` then started from zero and passed **100/100, fail=0** in 791 seconds on precisely that EFI. `build/stability-receipt.txt` reads `e534f7bada1ce1f61a300e659101415a1ed79874108a7021c1e02eea91d034d5 100/100`.

`python3 tools/checkpoint_bundle.py phase85-complete build/phase85-qualified-suite.log` checked this exact receipt, verified SHA-256 `c6ba568e799d3b65f8a37fe0ceb630f4a1fb2ac6f619c23ee7c57586e79f84cd` for the deployable archive, independently extracted the firmware/ESP/formatted AFS1 disk and **booted it cleanly in QEMU**. See `docs/RUNNING.md` for local QEMU instructions. The two earlier obsolete-harness stability runs were recorded as failures and never added to this 100/100. A separate earlier pre-kernel OVMF stall was observed and logged as a failure; explicit ESP selection hardens the harness, but its original cause is not established. No failed or interrupted run was promoted to a pass.
