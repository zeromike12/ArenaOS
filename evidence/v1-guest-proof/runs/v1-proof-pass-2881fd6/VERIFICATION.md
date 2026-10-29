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
| `evidence.json` | Harness output: per-checkpoint results, identities, install acknowledgment, screenshot inventory |
| `install-ack.json` | Captured installation acknowledgment from the guest protected installer |
| `c-gfx-app.apb1` | Staged APB1 bundle (36376 bytes, version=42) |
| `shots/cp01-...cp07.ppm` | 8 checkpoint screendumps (800x600 PPM) |
| `MANIFEST.sha256` | SHA-256 checksums of every file above |
| `VERIFICATION.md` | This file |

## Missing files (limitation, recorded explicitly)

The following files referenced by the original report are **not present**.
The QEMU binaries (`/home/user/phase16-backend/qemu-patched` wrapper and
`/home/user/phase15-pkg/backend/qemu-system-x86_64-patched` ELF) are no
longer available in this workspace, so a fresh QEMU run cannot be performed
to regenerate them. The original run's /tmp output directory is ephemeral and
has been recycled.

Per C's review directive item 1: this limitation is recorded explicitly. The
existing run is preserved unchanged. No historical output is reconstructed or
substituted. A new focused run will be performed when QEMU is available and
labeled as a replacement run.

- `v1-guest-serial.log` — Guest serial console output (lifecycle +
  resource receipts). NOT PRESENT.
- `qemu-stderr-v1-guest-proof.log` — QEMU stderr (kept, not discarded).
  NOT PRESENT.

## Checkpoint results

| # | Name | Result |
|---|------|--------|
| 1 | launch-ready Desktop | PASS |
| 2 | first native C window | PASS |
| 3 | keyboard response (fresh flash + event ack after keyboard input) | PASS |
| 4 | mouse response (fresh flash + event ack after pointer input) | PASS |
| 5 | two concurrent instances: focus/input isolation (both directions) | PASS |
| 6 | window removal with resource accounting (2 retired, Desktop restored, settled) | PASS |
| 7 | final V1 demonstration (visible flash) | PASS |

Overall: **PASS** (single verdict computation; no NOT RUN, no FAIL).

## Provenance (identities as recorded at run time)

- Tested source commit: `2881fd619e7bc315746c860e2ac5cff6d339f513`
- Code/commit for this package: `25d2c38` (hardening commit on top of `2881fd6`)
- Working tree: dirty (proof-infrastructure fixes — see git diff)
- QEMU wrapper: `/home/user/phase16-backend/qemu-patched` (hash `92ec1c12…`)
  — path no longer accessible in this workspace
- QEMU emulator ELF: `/home/user/phase15-pkg/backend/qemu-system-x86_64-patched`
  (hash `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff`)
  — path no longer accessible in this workspace
- Loader: `/home/user/glibc-2.40/lib/ld-linux-x86-64.so.2` — path no longer accessible
- APB1 sha256: `8b3e365e870e30324aa2752cf629e15c8524a15da22e69e3c1e773f7bb5d1916`
- ESP sha256: `ddc1191b281d52ac00e42ad9f6b3dfeaead275e5fec7ccbbf7f40681bbc3d8d6`
- Serial sha256: see `evidence.json` (serial log file itself not present)

## Checksum verification

From this directory:

    sha256sum -c MANIFEST.sha256

Every listed file must print "OK". The APB1 hash in `evidence.json`
(`apb1_sha256`) must match `c-gfx-app.apb1` and the install acknowledgment.
`v1-guest-serial.log` and `qemu-stderr-v1-guest-proof.log` are intentionally
omitted from the manifest (they are missing; see limitation above).

## Re-running the proof

QEMU is currently unavailable in this workspace. When it is restored:

    export ARENA_QEMU="/home/user/phase16-backend/qemu-patched"
    export RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo
    export ARENA_OVMF_CODE=/path/to/edk2-x86_64-code.fd
    export ARENA_OVMF_VARS=/path/to/edk2-i386-vars.fd
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
