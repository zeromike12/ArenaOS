# V1 Guest Proof — Passing Run (commit 2881fd6)

## What this package is

Durable evidence package for the V1 "Native Graphical Launch Foundation"
guest proof, run against source commit `2881fd619e7bc315746c860e2ac5cff6d339f513`
on branch `arena/f6b7ebc4-arenaos`. Result: **V1 GUEST PROOF PASS** — all 7
checkpoints PASS and the single overall verdict computes to PASS.

This is TEST INFRASTRUCTURE evidence only; nothing inside ArenaOS knows about
the proof script. The guest sees QMP input events and screendumps exactly as a
user would.

## Contents

| File | Description |
|------|-------------|
| `evidence.json` | Harness output: per-checkpoint results, identities (commit, QEMU, APB1, ESP, serial hashes), install acknowledgment, screenshot inventory |
| `v1-guest-serial.log` | Guest serial console output (lifecycle + resource receipts) |
| `install-ack.json` | Captured installation acknowledgment from the guest protected installer |
| `c-gfx-app.apb1` | Staged APB1 bundle (36376 bytes, version=42) |
| `qemu-stderr-v1-guest-proof.log` | QEMU stderr (kept, not discarded) |
| `shots/cp01-...cp07.ppm` | 8 checkpoint screendumps (800x600 PPM) |
| `MANIFEST.sha256` | SHA-256 checksums of every file above |
| `VERIFICATION.md` | This file |

## Checkpoint results

| # | Name | Result |
|---|------|--------|
| 1 | launch-ready Desktop | PASS |
| 2 | first native C window | PASS |
| 3 | keyboard response (fresh flash after keyboard input) | PASS |
| 4 | mouse response (fresh flash after pointer input) | PASS |
| 5 | two concurrent instances: focus/input isolation (both directions) | PASS |
| 6 | window removal with resource accounting (2 retired, Desktop restored, settled) | PASS |
| 7 | final V1 demonstration (visible flash) | PASS |

Overall: **PASS** (single verdict computation; no NOT RUN, no FAIL).

## Provenance (identities)

- Tested source commit: `2881fd619e7bc315746c860e2ac5cff6d339f513`
- Working tree: dirty (proof-infrastructure fixes only — see git diff)
- QEMU binary: `/home/user/phase16-backend/qemu-patched` (wrapper → QEMU 11.1.2 TCG ELF at `/home/user/phase15-pkg/backend/qemu-system-x86_64-patched`, `e0ea2b1f…f26ff`)
- QEMU binary sha256 (wrapper invoked): `92ec1c12cc174d549dfdbedf921984784252d66630bdeca32d248085b42c76a1`
- APB1 sha256: `8b3e365e870e30324aa2752cf629e15c8524a15da22e69e3c1e773f7bb5d1916`
- ESP sha256: `ddc1191b281d52ac00e42ad9f6b3dfeaead275e5fec7ccbbf7f40681bbc3d8d6`
- Serial sha256: see `evidence.json` / `MANIFEST.sha256`

## Checksum verification

From this directory:

    sha256sum -c MANIFEST.sha256

Every listed file must print "OK". The APB1 hash in `evidence.json`
(`apb1_sha256`) must match `c-gfx-app.apb1` and the install acknowledgment.

## Re-running the proof

    export ARENA_QEMU="/home/user/phase16-backend/qemu-patched"
    export RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo
    cd /home/user/ArenaOS
    python3 tools/v1_guest_proof.py

A fresh run-specific output directory is created under /tmp each run; the
overall verdict line is `V1 GUEST PROOF PASS` when all conditions are met.

Focused (no-QEMU) controls:

    python3 tools/v1_guest_proof.py --controls-only

## Notes

- The original `2881fd6` run artifacts remain preserved unchanged at
  `evidence/v1-guest-proof/` (the pre-fix run, checkpoint 7 FAIL).
- No merge, release, or V1 acceptance declaration is made by this package.
- No V2/V3/audio/GPU/SDL3 work is included.
