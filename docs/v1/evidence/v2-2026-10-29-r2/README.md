# v2-2026-10-29-r2 — V1 qualification evidence

Produced 2026-10-29T23:26:10Z from commit
`65af1d89a5254cd6a45cb96341966f9265a42742` on branch `arena/af044828-arenaos`
(working tree dirty: `True`).

## What is here

| Kind | Count | Notes |
|---|---|---|
| Serial logs | 31 | The guest's own output under QEMU TCG |
| Screenshots | 32 | The exact frames each checkpoint assertion was made on (PNG, losslessly re-encoded from the QEMU screendump raster) |
| Manifests and receipts | 45 | JSON verdicts, identities, build receipts |

## How to check it

Extract the archive anywhere and verify every recorded digest from a fresh
directory (nothing is compared against the original workspace):

```sh
tar xzf v2-2026-10-29-r2.tar.gz
cd v2-2026-10-29-r2 && python3 - <<'PY'
import hashlib, json, pathlib
root = pathlib.Path("v2-2026-10-29-r2")
m = json.loads((root / "MANIFEST.json").read_text())
bad = []
for rel, want in m["files"].items():
    p = root / rel
    if not p.is_file():
        bad.append((rel, "missing"))
        continue
    got = hashlib.sha256(p.read_bytes()).hexdigest()
    if got != want["sha256"] or p.stat().st_size != want["size"]:
        bad.append((rel, got + " != " + want["sha256"]))
print("files:", len(m["files"]), "mismatches:", len(bad))
for b in bad:
    print(" ", b)
raise SystemExit(1 if bad else 0)
PY
```

`tools/v1_evidence.py verify <archive>` performs exactly this check, plus a
scan for files that exist in the archive but not in the manifest.

`package` refuses to run unless every guest run in this directory publishes
all nine checkpoint images, each PNG still decoding to the raster digest the
harness recorded when that checkpoint passed.

PPM working copies are not shipped (70 of them): a PPM is a fixed
header plus the identical raster, so it is reproducible from the published
PNG, and each tested PPM's own SHA-256 is recorded in its run's
`evidence.json`.

## How the verdicts were produced

- `guest-gcc/`, `guest-clang/`: `tools/v1_guest_proof.py` — eight
  checkpoints; `evidence.json` is the run's own verdict, `harness-output.log`
  is the transcript, `v1-guest-serial.log` is the guest serial, `shots/` are
  the frames, `qemu-stderr.log` is the emulator's stderr.
- `lifecycle-cycles/`: `tools/v1_cycles.py` — ten launch/close cycles, each
  with its own cursor, instance-specific retirement and exit receipts,
  settlement against the pre-cycle baseline, and visual removal.
- `env/`: environment recovery receipts (emulator identity, loader, TCG
  acceleration, toolchain versions, protected guards).

A missing serial log, a missing harness transcript, an absent identity or a
missing required artifact makes the run FAIL — see `docs/v1/V2-STATUS.md` §6.
