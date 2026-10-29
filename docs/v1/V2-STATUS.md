# V2 — V1 qualification status (durable)

This file is the standing record of what this session did, what it proved and
what it did **not** prove. It is written to be read by M and by C, and it is
deliberately conservative: a stage appears as PASS only with the command that
produced it, the artifact identity it ran against, and the evidence file that
holds its raw output.

Round 1 (2026-10-29, commit `65af1d8`) and round 2 (this round, same commit,
uncommitted working tree) are kept separate. Nothing in round 1's evidence
directory was rewritten.

---

## 1. Source checkpoint and working branch

| Item | Value |
|---|---|
| Branch | `arena/af044828-arenaos` (session branch; cannot be renamed) |
| Round-1 checkpoint | `65af1d89a5254cd6a45cb96341966f9265a42742` |
| Round-2 tree | same commit + uncommitted round-2 changes (see §4) |
| Merged into `main` | no — merge and release are M's decisions |
| History rewritten | no — no force-push, no rebase, no `reset` of published commits |

The round-2 working tree is uncommitted by design: it is the review
candidate. `tools/v1_gate_stage.py` stamps every gate receipt with the commit
and the dirty paths that ran, so a reader can tell exactly what was executed.

---

## 2. Round-2 summarised in one paragraph

Round 2 corrected four defects in the V1 evidence chain and extended the
matrix: (a) an input checkpoint can no longer pass on an acknowledgment
without a captured frame showing the response, and vice versa; (b) every
checkpoint now publishes the exact frame its assertion was made on, as a
lossless PNG whose raster digest is recorded, and packaging refuses a run
that is missing one; (c) cleanup compares 21 resource fields from one
production snapshot pair, with a single documented, bounded, one-directional
frame warm-up allowance; (d) the guest refusal matrix (21 cases) now runs in
the guest, through the production launch path, instead of only on the host.
The gate was reconciled with the claims it makes. The canonical EFI build
remains blocked by an environment limitation that predates this round (§4 B1)
and the artifacts used here are therefore explicitly **not** qualification
artifacts.

---

## 3. Environment (verified this round)

| Component | Identity | Evidence |
|---|---|---|
| QEMU (patched, TCG) | `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff` | `env/qemu-identities.txt` |
| Loader | private glibc 2.40 `ld-linux-x86-64.so.2`, `63704534fba7799d…` | `env/isolated-runtime.txt` |
| Firmware | code `7e34e3b8…`, vars `5d2ac383…` | `env/qemu-identities.txt` |
| GCC | 12.2.0 | `env/compilers/tool-identities.txt` |
| Clang route | Zig 0.17.0 `zig cc -target x86_64-freestanding-none` | idem |
| Rust | 1.97.0 (`2d8144b78`), LLVM 22.1.6 | `env/rust-cargo-version.txt` |

The emulator is only ever started with explicit `-accel tcg`; the identity is
resolved per link of the chain (wrapper, ELF, loader, runtime libraries,
firmware) rather than from a wrapper hash alone.

---

## 4. Findings and corrections (round 2)

| # | Finding | Correction | Where |
|---|---|---|---|
| F1 | The keyboard/mouse checkpoints accepted `ack["ok"]` alone: an acknowledgment proved delivery, never that anything was drawn | `evaluate_input_proof()` requires a fresh, correctly-scoped ack **and** a captured frame with the instance's flash inside its own rectangle, **and** a flash-free pre-frame; used by checkpoints 4, 5, 6 and 8 | `tools/v1_guest_proof.py` |
| F2 | `__ack-probe__` was overwritten by every input, so no single published file could represent several checkpoints | `save_frame()` publishes a per-checkpoint PNG plus the tested PPM; `REQUIRED_FRAMES` names them; the verdict fails when one is absent | idem |
| F3 | Evidence references pointed at `.ppm` files that were never published, and packaging checksummed whatever happened to exist | PNG writer in-tree (no new dependency), `frames` recorded with tested-PPM, published-PNG and raster digests; `tools/v1_evidence.py package` re-decodes every required PNG and compares it with the recorded raster | `tools/v1_guest_proof.py`, `tools/v1_evidence.py` |
| F4 | Cleanup compared only app instances and windows | 14 detail + 7 receipt fields compared from one adjacent production snapshot pair; controls prove a thread, capability or frame leak fails, and that a missing/stale receipt cannot read as settled | `tools/v1_guest_proof.py` |
| F5 | The refusal matrix had never executed in the guest | `tools/v1_refusals.py` launches the client through the protected installer and ordinary launch path; 21 cases, no window, no resource drift, exit 0, both compilers | `tools/v1_refusals.py`, `experiments/c-runtime/app/c_gfx_refusals.c` |
| F6 | **The refusal harness launched the wrong application.** The launcher filters by *substring* and Enter launches the first match, so typing `r` selected **Terminal**; the client never ran and the run looked like a client defect | The selection is proven by the captured frame `cp09-launcher-selection.png`, and the harness now types the unique substring `refusal` before Enter | `tools/v1_refusals.py` |
| F7 | `.gitignore` line 7 (`*.log`) silently excluded every serial and harness log from round-1's published evidence (0 `.log` files were committed) | Rule removed; evidence logs are committed, PPM working copies are not (they are a fixed header plus the published PNG's raster, and their digests are recorded) | `.gitignore` |
| F8 | `tools/v1_gate.sh` claimed to reproduce everything but never ran `tools/run_tests.sh`, and a partial run printed the same PASS as a full one | Stage receipts (`tools/v1_gate_stage.py`) + aggregate (`tools/v1_gate_aggregate.py`): only `--mode full` can print `V1 QUALIFICATION: PASS`; partial and unqualified runs say so and exit non-zero | `tools/v1_gate.sh` |

### 4.1 Blocker B1 — the canonical EFI build (unchanged, not caused by round 2)

`tools/build_phase14_pie.sh` recomputes the protected Phase-14 PIE fixture and
refuses when the digest is not
`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`. That
reproduction requires the **official** `rust-std` target components for
`x86_64-unknown-none` and `x86_64-unknown-uefi`
(`5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a`,
`2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0`). The
sandbox may only reach github.com, codeload/api.github.com,
registry.npmjs.org and pypi.org; `static.rust-lang.org` is refused at TLS
level. Rebuilding the sysroot from source produces a different crate
disambiguator (`Csi1tY5tVYnWh_` vs the official `CsieveXhe8AWz_`) and hence a
different fixture — investigated again this round (flag variants, `-C
panic`/opt-level/debug-assertions, `cargo -Z build-std`) without success; the
disambiguator is driven by bootstrap-grade crate metadata, not by codegen
flags.

The documented development substitute `tools/build_phase16_dev.sh` verifies
the committed fixture instead of rebuilding it. **Artifacts it produces are
explicitly not qualification artifacts** (§7).

---

## 5. What round 2 executed (raw results)

| Stage | Command | Result | Artifact |
|---|---|---|---|
| Focused controls | `python3 tools/v1_guest_proof.py --controls` | **35/35 PASS** | — |
| Guest proof, GCC | `V1_EVIDENCE_DIR=…/guest-gcc python3 tools/v1_guest_proof.py` | **8/8 checkpoints PASS** | EFI `7134acf1…` |
| Guest proof, Clang | same, `V1_CC=clang` | **8/8 checkpoints PASS** | idem |
| Refusal matrix, GCC | `V1_EVIDENCE_DIR=…/refusals-gcc python3 tools/v1_refusals.py` | **21/21 cases PASS** | idem |
| Refusal matrix, Clang | same, `V1_CC=clang` | **21/21 cases PASS** | idem |
| Lifecycle | `V1_EVIDENCE_DIR=…/lifecycle-cycles python3 tools/v1_cycles.py 10` | **10/10 cycles PASS** | idem |
| Boot stability | `bash tools/stability_loop.sh 100` | **100/100 boots green** (1056 s) | idem |
| Formal regression | `bash tools/run_tests.sh` | **not completed** — host suites ran with 0 failures for ~18 min, then terminated to free the emulator (log ends `EXIT=143`) | idem |
| Gate (guest mode) | `V1_GATE_MODE=guest bash tools/v1_gate.sh` | **all 10 stages PASS**; aggregate: `V1 QUALIFICATION: BLOCKED` (dev-path build) | EFI `7134acf1…` |
| Evidence package | `python3 tools/v1_evidence.py package …` | 108 files, 440 918 B | sha256 `26674d87…f66` |
| Fresh-extraction verify | `python3 tools/v1_evidence.py verify …` | 108 files, **0 problems** | idem |

### 5.1 Formal regression (incomplete this round)

`regression/run-tests.log` records the run exactly as it happened: the host
suites executed in order with no failures, and the run was terminated after
about eighteen minutes (the log's last line is `EXIT=143`) to release the
emulator for the guest gate in the same session. It did **not** reach the
guest suites, so it is not a regression result and nothing is inherited from
it. Two further facts belong here:

* round 1's last complete run was `ALL TESTS PASSED (121 test suites)` at
  commit `bdf6c66`, recorded in `docs/v1/evidence/v2-2026-10-29/`;
* thirteen of the 121 sub-suites rebuild the kernel through
  `tools/build.sh`, so they cannot pass while B1 stands. Their failure is
  environmental, and the final gate run must name them as such rather than
  count them as regressions.

The full gate (`V1_GATE_MODE=full`) requires this stage; it is therefore
**not** claimable until B1 is resolved and the suite completes.

---

## 6. Qualification matrix

"Required" is the contract; "state" is what exists today.

| # | Requirement | State | Evidence |
|---|---|---|---|
| 1 | Ack **and** visible response, produced by production logic | **DONE** | `guest-*/evidence.json` (`input_proof` per checkpoint), 8 controls |
| 2 | Actual successful checkpoint frames published | **DONE** | `guest-*/shots/cp0*.png`, `frames` digests, packaging validation |
| 3 | Cleanup compares all enforceable resources | **DONE** | `lifecycle-cycles/cycles-evidence.json`, 5 cleanup controls |
| 4 | Guest refusal matrix, cases 11–17 | **DONE** (HOST-ONLY + GUEST) | `refusals-*/refusals-evidence.json`, 21 cases |
| 5 | Final-artifact stability (100/100 boots) | **DONE for this artifact**, but the artifact is dev-path (§4.1) | `stability/stability-100.log` |
| 5b | EFI rebuild reproducibility | **BLOCKED (B1)** | build log: `reproducible PIE fixture hash changed` |
| 5c | ESP ↔ embedded-EFI identity | **DONE** | `env/`, `evidence.json` `image.boot_efi_sha256` |
| 6 | Gate command matches the claims | **DONE** | `tools/v1_gate.sh` + receipts + aggregate |
| 7 | Final review candidate + package | **DONE** (§7) | archive + `.sha256` + fresh-extraction verify |

Artifact identities for this round: EFI
`7134acf11212ddd4845e0b29ca8058a84d3e9944d2da92b6ab7efd9588806cb5`, ESP
`d74b9773b300ada1341a81c34551df90a7389caa2b3f4ef10f7de8f282329d30`; GCC
client ELF `07f9fbc5d7dc…`, APB1 `073f08f79f3b…`; Clang client APB1
`c0da952974c6…`; refusal APB1 `841d086b3b52…` (gcc).

**Qualification status: NOT COMPLETE.** The V1 matrix is not fully green
because requirement 5b cannot be executed in this sandbox. Everything that
could be executed has been, and the artifacts are labelled for what they are.

---

## 7. Evidence package and classification

| Path | Class | Contents |
|---|---|---|
| `docs/v1/evidence/v2-2026-10-29/` | GUEST + HOST, round 1 | unchanged, do not edit |
| `docs/v1/evidence/v2-2026-10-29-r2/` | GUEST + HOST, round 2 | this round's runs, receipts, frames, logs |
| `docs/v1/evidence/v2-2026-10-29-r2.tar.gz` | package | 108 files, 440 918 B, sha256 `26674d877692a4fd66330643cbf2f862381d5d1e5d86a1398b95b9b6e502cf66`; verified from a fresh extraction, 0 problems |
| `…-r2/gate/receipts/*.json` | receipts | per-stage identity: commit, dirty paths, EFI/ESP digests, newest tracked source, exit code |

Classification of the artifacts used this round:

- `DEV-PATH` — built by `tools/build_phase16_dev.sh`: the protected Phase-14
  fixture was **verified, not rebuilt**; the bare-metal sysroot came from
  `tools/build-sysroot.sh`. EFI `7134acf1…`, ESP `d74b9773…`. These are the
  only artifacts available while B1 stands, and they are **not** qualification
  artifacts, not release candidates and not Phase 14/15 identity
  reproductions.
- `HOST-ONLY` — host-side suites (C runtime, AFS2, packaging); they prove
  logic, not guest behaviour.
- `GUEST` — executed inside ArenaOS under the pinned QEMU with explicit TCG.

---

## 8. Known limitations (stated, not hidden)

1. **B1**: the canonical `tools/build.sh --image` cannot complete; the EFI/ESP
   used here are dev-path builds. The decision needed is one of: allowlist
   `static.rust-lang.org`, supply the two component archives (digests above),
   or accept a dev-path artifact as the candidate with the caveat recorded.
2. **Strict Phase-15 gate**: `PHASE15_STRICT_GATE=1` pins the historical
   EFI/ESP identities, so a rebuilt candidate can never pass strict mode
   without editing those pins. Round 2 did not edit them.
3. **Round-1 artifacts are gone**: the previous sandbox's `build/` directory
   is not preserved, so the round-1 EFI (`bcd86866…`) cannot be re-verified;
   only its recorded identity in the round-1 evidence remains.
4. **Runtime heap warm-up**: `free-frames` may settle up to 32 frames below
   the baseline (measured: 5 after the first lifecycle, 3 more with two
   concurrent instances). The allowance is one-directional, bounded, recorded
   per run, and every other field must be exactly equal.
5. **Launcher semantics**: the app launcher filters by substring and launches
   the first match; harness code must type a unique substring. This bit the
   refusal harness once (F6) and is now proven by a captured frame.
