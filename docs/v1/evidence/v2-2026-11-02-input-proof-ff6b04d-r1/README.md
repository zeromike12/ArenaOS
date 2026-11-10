# V1 input-proof retry diagnostic — diagnostic only

> **Qualification claim: NONE. V1 is not accepted.** The full r6 gate remains **FAIL**; r6 is not replaced by the later standalone guest-proof retry. This archive preserves attempts and diagnostic context—it is not a qualification package.

## Attempt outcomes

| Attempt | UTC start (run label) | Standalone guest-proof result | Checkpoint 5 |
|---|---|---:|---:|
| `guest-run/` | 2026-11-02 04:43:14Z | FAIL | FAIL: fresh acknowledgment, no visible in-window response |
| `guest-run-retry1/` | 2026-11-02 04:48:06Z | FAIL | FAIL: fresh acknowledgment, no visible in-window response |
| `guest-run-retry2/` | 2026-11-02 04:51:13Z | PASS for the eight standalone checkpoints only | PASS: acknowledgment and visible response in the window |

All attempts record the same APB1 SHA-256: `cedb9591938a935caa90a0f0bf4ed9accc8eff7f4e37e663ad2b3d8482a431e0`. Retry2’s mouse acknowledgment and visible response are recorded at `2026-11-02T04:51:42.836239751Z`. The original and retry1 failures remain failures; retry2 does not erase them or establish full V1 acceptance.

The predicate and 4-second timeout were not relaxed. Checkpoint 5 still requires a fresh same-instance/event acknowledgment, a visible response inside the target window, and a clean pre-input frame. The saved-frame scan analysis found roughly 1.0–1.14 seconds per no-flash sample with the pure-Python scanner versus about 0.028 seconds with NumPy 2.4.6; the recorded predicate booleans match in both paths. This supports a sampling-latency explanation but does not prove when an unseen response began. No captures were invented.

## Missing frames and retained captures

`missing-frames-r1.json` explicitly lists the **12 required checkpoint PNGs** absent from the two failed runs (six each, cp05 through cp08 including the three cp06 image entries). The failed cp05 timestamped probe PNGs remain present and are listed separately; they do not substitute for the missing checkpoint summary frames. Retry2 has all required checkpoint PNGs.

`raw-evidence-inventory-r2.json` lists every file under the three attempt directories with size and SHA-256. It records 44 PNGs and 26 PPMs. The 26 restored PPMs were reconstructed only from saved lossless PNG rasters and accepted only when each matched its originally recorded PPM SHA-256 (`capture-restoration-verification-r1.json`). Initial-attempt raw QEMU boot streams are retained. Retry1 and retry2 have harness serials, screenshots, run records, EFI/APB1 files, and transcripts, but no per-boot `raw-boots/` directories are present in this resumed workspace. The raw proof-boot serial and harness serial differ; both are preserved without normalization.

## Fresh current-source build and source identity

A fresh `tools/build.sh --image` build was completed from current committed source `b2c23fb5cf35dae29ff845bec82eb2c001028c3f`, with source paths clean and the unchanged Phase-14 hash guard passing. The build’s image freshness check reports `fresh: true`.

- EFI: 11,039,232 bytes, SHA-256 `b2fc4a295bfa5ef33c141f2fcfd099af5a761c25250592371996a02d6eba377a`
- ESP: 16,777,216 bytes, SHA-256 `a2f9821206c8efb22f13a6b3bcb92d5c6bd30b4bd60cef9bf3673b6280950551`

Both hashes exactly match the EFI/ESP identities recorded for the three historical QEMU attempts. Fresh copies are in `build-inputs/current-b2c-final-r1/`; the earlier recovered EFI is kept separately. This establishes artifact identity continuity, **not** a substituted source-commit identity.

The run records cite source commit `ff6b04d791aeabd3e5875194460153c42580f742`, but that Git object is absent from this resumed checkout. The source patch was preserved in current commit `b2c23fb`; full-tree equivalence to the unavailable run commit cannot be independently checked. No new QEMU run was made here because the pinned QEMU wrapper/ELF, isolated glibc loader, and OVMF files are absent. Their recorded hashes and the missing historical Phase-15 EFI/ESP inputs are itemized in `omitted-inputs-r1.json`; they are not represented as recovered.

## Independent qualification status

The r6 diagnostic receipt and aggregate reference are preserved under `r6-reference/`. **The full r6 gate remains FAIL; requirement 4b remains OPEN**, with no genuine stale/foreign capability launch and no M-recorded waiver. Phase 13 remains at its existing **600-second** timeout and expected behavior; no timeout increase or skip was made. No waiver, merge, release, or V1 acceptance is implied.

## Integrity verification

The archive includes `MANIFEST.json`, this README, `DIAGNOSTIC-REPORT.json`, the missing-frame inventory, omitted-input inventory, raw run evidence, build identities, logs, restoration records, and a selected current-source snapshot. Verify archive integrity from a fresh extraction with:

```sh
python3 tools/v1_diagnostic_evidence.py verify v2-2026-11-02-input-proof-ff6b04d-r1-diagnostic.tar.gz
```

This checks only diagnostic archive integrity. It does not run strict qualification validation and cannot change the r6 or V1 qualification verdict.
