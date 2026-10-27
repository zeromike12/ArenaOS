# Sealed correction evidence (2026-10-27)

This directory makes the raw evidence for the PR #6 review-correction pass
retrievable from the repository. The compact TSV receipts elsewhere are
indexes; this archive is the evidence bundle.

- Archive: `arenaos-testing-correction-evidence-30fe810.tar.gz`
- Size: 36,874,615 bytes compressed; 762,516,770 bytes of staged files
  (777,953,280-byte tar stream).
- SHA-256:
  `9fccf05b686fe0683ca30fef6f010a998b537adc449f5d267f2403da94c5b6ea`
- Sidecar: `arenaos-testing-correction-evidence-30fe810.tar.gz.sha256`
- Tested source: `30fe81047ff4a5c44a09d959bb92b7558c7891e9`
- Fresh-extraction result: `EXTRACT-VERIFY.log` (checksum, 6,317-file
  manifest, and six complete extracted-run `verify-run` checks).

The archive contains the complete corrected full run
`20261027T183025Z-full-30fe810-434497-768c5a` (PASS 138/138), the two
preceding failed full attempts' run-level records and failing attempts,
focused Phase 13/archive-preflight/M9-repeat runs, backend controls,
quick/infrastructure controls, representative cache hit/miss/invalidation
evidence, environment identities, timing snapshots, a manifest, and
reproduction instructions in its top-level `README.md`.

Verify it without dirtying a checkout:

```sh
mkdir -p /tmp/arena-evidence-check
cp arenaos-testing-correction-evidence-30fe810.tar.gz* \
  /tmp/arena-evidence-check/
cd /tmp/arena-evidence-check
sha256sum -c arenaos-testing-correction-evidence-30fe810.tar.gz.sha256
mkdir extracted
tar -xzf arenaos-testing-correction-evidence-30fe810.tar.gz -C extracted
python3 extracted/arenaos-testing-correction-evidence-30fe810/verify_manifest.py \
  extracted/arenaos-testing-correction-evidence-30fe810
```

Then follow the archive README's tested-source `verify-run` commands.
Do not reseal this archive merely to embed its own hash: the sidecar is
the external checksum of record.
