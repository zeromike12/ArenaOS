# V1 Guest Proof — Review Correction for Commit 2881fd6

## Status: DEVELOPMENT EVIDENCE — QUALIFICATION VERDICTS INVALIDATED

The seven-checkpoint PASS reported in commit `2881fd6` does **not** establish
full V1 qualification. This run is classified as **development evidence** with
**invalidated qualification verdicts**. Useful observations are preserved; no
original receipt or capture has been silently rewritten or discarded.

## Preserved Original Artifacts (commit 2881fd6)

| Artifact | Path | Notes |
|---|---|---|
| Serial log | `evidence/v1-guest-proof/v1-guest-serial.log` | Original guest serial output, unchanged |
| Evidence JSON | `evidence/v1-guest-proof/evidence.json` | Original evidence, unchanged |
| Checkpoint captures | `evidence/v1-guest-proof/cp01-*.ppm` … `cp07-*.ppm` | Original PPM screenshots, unchanged |
| Install acknowledgment | `evidence/v1-guest-proof/install-ack.json` | Original install ack, unchanged |

## Known Defects in the Original Run

### CP6: Historical-snapshot false positive (Item 2)

`await_settlement()` searched the **entire** serial history for any
zero-instance/zero-window resource receipt. The initial Desktop baseline
(line 1112: `app-instances=0, windows=0`) satisfies that search, so CP6 passed
without ever verifying post-close settlement.

The committed log confirms:
- Line 1112: baseline `app-instances=0, windows=0` (pre-launch)
- Line 1594-1595: instance 1 live, `app-instances=1, windows=1`
- Line 1797-1798: instance 2 live, `app-instances=2, windows=2`
- Line 1808: one process exit recorded
- Line 1811: application retirement acknowledged
- Line 1814: post-close `app-instances=1, windows=1` — **NOT** zero/zero
- Line 2010-2011: final demonstration relaunched, `app-instances=2, windows=2`

CP6 accepted the historical baseline snapshot rather than fresh cleanup
evidence. The committed verdict `PASS` for CP6 is **invalidated**.

### CP3–CP5: Generic flash inspection, not event-specific (Item 3)

- CP3–CP5 inspect generic flashes without consuming instance-specific event
  counters or acknowledgments.
- `await_no_flash()` results are ignored in places.
- CP4 injects its click before attempting to settle the prior flash.
- No verification that the flash identifies the event type and intended
  instance.

### CP5: Single-direction isolation only (Item 4)

- Only one focus direction tested (focus second instance, type into it).
- Connected components used as instance identities — unreliable with
  overlapping/flashing windows.
- No reverse-direction test (focus first instance, verify second unchanged).

### Overall success computation (Item 5)

- `main()` writes `all_pass=True` **before** calculating checkpoint success.
- Missing required identities (QEMU SHA-256 = `"MISSING"`) do not prevent
  PASS.
- No single production verdict computation shared by exit status, console
  summary, and evidence JSON.

### Build/backend provenance incomplete (Item 6)

- Source commit recorded as `d2bfd66…` (pre-fix commit, not the tested fix
  commit).
- Working tree marked dirty.
- QEMU binary SHA-256 = `"MISSING"`.
- No compiler identities, no ELF/APB1/EFI/ESP hashes, no firmware/loader
  hashes.

### Screenshots not in retrievable submission (Item 7)

- Screenshot hashes recorded in evidence JSON but the PPM files themselves
  were not committed to a durable, retrievable location.

## Corrections Applied (subsequent commits)

The following defects are corrected in the production harness on this branch:

1. **CP6**: Serial cursor captured before each close; only fresh lifecycle and
   resource receipts after the cursor are evaluated; specific instances
   tracked; fresh zero-instance/zero-window settlement required after all
   test instances close; visual window removal required.

2. **CP3–CP5**: Event-specific input sequence — settle prior state, capture
   pre-input counters, inject specific event, require new event-specific
   acknowledgment correlated with visible response.

3. **CP5**: Two-direction isolation — focus A then B, each direction verified
   independently with instance identity established by geometry, not just
   connected components.

4. **Overall success**: Single verdict computation used by exit status,
   console summary, and evidence JSON; success requires every checkpoint
   PASS, required identities present, required evidence files valid.

5. **Provenance**: Exact tested commit recorded; compiler identities; ELF,
   APB1, EFI, ESP hashes; emulator/firmware/loader identities resolved.

6. **Evidence**: Original captures published in durable evidence package with
   captions, serial, QEMU stderr, harness output, manifest, checksums.

## Focused Negative Controls Added (no QEMU)

| Control | Purpose |
|---|---|
| Zero-resource receipts but app live | CP6 must FAIL when serial has zero/zero but app-instances > 0 |
| Only one of two instances retires | CP6 must FAIL when only one instance retires |
| Windows gone but resources leak | CP6 must FAIL when visual windows gone but app-instances > 0 |
| Resources settle but stale window | CP6 must FAIL when resources zero but visual window remains |
| Fresh settlement receipts missing | CP6 must FAIL when no fresh zero/zero receipt after cursor |
| All-success control | CP6 must PASS when retirement + fresh zero/zero settlement |

## Classification

This run (`2881fd6`) is **development evidence only**. The qualification
verdicts it reports are **invalidated** by the defects above. Full V1
qualification requires a new candidate run with the corrected harness, all
7 checkpoints PASS plus negative controls, complete provenance, and published
evidence.

V1 acceptance, merge, and release remain pending.
