# V2 — V1 qualification status (durable)

This file is the standing record of what this session did, what it proved and
what it did **not** prove. It is written to be read by M and by C, and it is
deliberately conservative: a stage appears as PASS only with the command that
produced it, the artifact identity it ran against, and the evidence file that
holds its raw output.

Round 1 (2026-10-29, commit `65af1d8`) and round 2 (`c4d7cf1` plus this
round's follow-up changes) are kept separate. Nothing in round 1's evidence
directory was rewritten.

---

## 1. Source checkpoint and working branch

| Item | Value |
|---|---|
| Branch | `arena/af044828-arenaos` (session branch; cannot be renamed) |
| Round-1 checkpoint | `65af1d89a5254cd6a45cb96341966f9265a42742` |
| Round-2 checkpoint | `c4d7cf13e2a958e5e8e28a2f91aff143f0d79c7e` |
| Round-2 tree | that commit + the follow-up changes listed in §4.2 |
| Merged into `main` | no — merge and release are M's decisions |
| History rewritten | no — no force-push, no rebase, no `reset` of published commits |

The gate receipts stamp every stage with the commit, the dirty paths, the
EFI/ESP digests **and a digest of the tracked code as it existed when the
stage ran** (`code_fingerprint`). The aggregate compares that digest, not the
commit id: committing work that was already on disk changes nothing that
executed, while an uncommitted edit changes everything. Control A and control
B in `gate/gate-controls.txt` demonstrate both directions.

---

## 2. Round 2 in one paragraph

Round 2 corrected the defects M raised and extended the matrix: (a) an input
checkpoint can no longer pass on an acknowledgment without a captured frame
showing the response, or on a frame without a fresh ack; (b) every checkpoint
publishes the exact frame its assertion was made on, as a lossless PNG whose
raster digest is recorded, and packaging refuses a run that is missing one or
whose archive does not match its manifest; (c) cleanup compares 21 resource
fields from one production snapshot pair, with a single documented, bounded,
one-directional frame warm-up allowance, and controls prove a thread,
capability or frame leak fails the verdict; (d) the guest refusal matrix (21
cases) runs in the guest through the production launch path for both
compilers; (e) the gate was reconciled with the claims it makes, by receipts
that identify the code by content and an aggregate that will not print a
qualification for a partial or unqualified run. Two things are **not** done
and are not dressed up as done: the canonical EFI build is blocked by an
environment limitation that predates this round (§4.1 B1), and the formal
121-suite regression is 22/121 with the other 99 stopped at that same guard
(§5.1). The artifacts used here are explicitly **not** qualification
artifacts, and V1 acceptance remains M's decision.

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
| F9 | `settle_snapshot()` could be called in the gap between the Desktop's receipt line and its detail line, returning "no settled snapshot" for a machine that had just printed one; the Clang refusal cell failed this way once (23:29Z run) | `wait_settle_snapshot()` waits (bounded, 15 s) for a *complete, adjacent* pair; it relaxes nothing — a Desktop that never prints a pair still fails | `tools/v1_guest_proof.py`, `tools/v1_refusals.py`, `tools/v1_cycles.py` |
| F10 | The evidence archive was built by adding the whole directory, so it shipped the PPM working copies the manifest deliberately omits; `verify` then reported an unlisted file | The archive is built from the manifest plus `MANIFEST.json`/`README.md` only, so archive contents and manifest are the same set by construction | `tools/v1_evidence.py` |
| F11 | Gate receipts were tied to the commit id, so a documentation commit invalidated a genuine run and an uncommitted code edit did not; `--mode quick` had no stage list | Receipts carry `code_fingerprint` over tracked non-doc files and the aggregate compares that; `quick` has its own stage list and is labelled partial | `tools/v1_gate_stage.py`, `tools/v1_gate_aggregate.py` |
| F12 | The refusal harness kept a `V1_REFUSAL_ELF` override that let any ELF stand in for the built client | Removed; the guest case runs the client the build just produced | `tools/v1_refusals.py` |
| F8 | `tools/v1_gate.sh` claimed to reproduce everything but never ran `tools/run_tests.sh`, and a partial run printed the same PASS as a full one | Stage receipts (`tools/v1_gate_stage.py`) + aggregate (`tools/v1_gate_aggregate.py`): only `--mode full` can print `V1 QUALIFICATION: PASS`; partial and unqualified runs say so and exit non-zero | `tools/v1_gate.sh` |

### 4.2 What changed after the first round-2 commit

`c4d7cf1` published the corrected evidence chain. The follow-up work in this
tree is: F9 (a real race that made one refusal cell fail), F10 (archive vs
manifest), F11 (receipt identity, `quick` mode), F12 (the override). Every
guest stage was re-executed after those edits, and the numbers in §5 are from
that re-execution, not from `c4d7cf1`'s run.

### 4.1 Blocker B1 — the canonical EFI build (**RESOLVED in round 3; see §9**)

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
| Formal regression | `bash tools/run_tests.sh` | **22 passed / 99 failed**, single root cause: `tools/build.sh --image` → PIE guard (B1). See §5.1 | idem |
| Gate (guest mode) | `V1_GATE_MODE=guest bash tools/v1_gate.sh` | **all 10 stages PASS** (2 independent runs); aggregate: `V1 QUALIFICATION: BLOCKED` (dev-path build) | EFI `7134acf1…` |
| Gate (full mode) | `V1_GATE_MODE=full bash tools/v1_gate.sh` | **not claimable**: the regression stage exits 1 (B1) | — |
| Gate self-controls | `gate/gate-controls.txt` | stale code → FAIL exit 1; docs-only edit → receipts still accepted | — |
| Evidence package | `python3 tools/v1_evidence.py package …` | 111 files, 461 677 B | sha256 `07ff4d44…148` |
| Fresh-extraction verify | `python3 tools/v1_evidence.py verify …` | 111 files, **0 problems** | idem |

### 5.1 Formal regression — executed, 99 suites blocked by B1

`regression/run-tests.log` is the complete transcript of one run of
`bash tools/run_tests.sh` on this candidate. It ran to completion; the result
is **22 of 121 suites passed, 99 failed**, and every one of the 99 traces to
a single line: `tools/build.sh --image` exits non-zero at
`tools/build_phase14_pie.sh`, which refuses because the locally built
`rust-std` for `x86_64-unknown-none` produces the PIE fixture digest
`b8636bd4…` instead of the protected `d7b0a8cf…` (blocker B1, §4.1).

Two of those failures need naming because they look like product failures and
are not:

* `test_m9_font_red.py` prints only a summary line; its two sub-runs
  (`build/m9-font-red-guest.log`, `build/m9-font-restored-guest.log`) both end
  in the same `CalledProcessError` from `tools/build.sh --image`;
* the Phase-13 and Phase-14 guest suites fail in `mtest.build()` before they
  ever start an emulator.

So the run is **not** a regression signal either way: it never reached the
product behaviour in those 99 suites. It is reported with its exact numbers
rather than inherited from round 1, whose `ALL TESTS PASSED (121 test suites)`
at `bdf6c66` was produced by an environment that still had the official
`rust-std` components — the component this sandbox cannot fetch (§4.1). **A
historical pass is not evidence for a changed or differently built artifact**,
which is the same rule that forbids counting ten cycles as a hundred boots.

Consequence for the gate: `V1_GATE_MODE=full` cannot pass while B1 stands,
because it requires this stage. The `regression` receipt records `exit 1` and
the note above; the aggregate reports the full gate as FAIL rather than
laundering 22 passes into a qualification.

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
| 5b | EFI rebuild reproducibility | ~~BLOCKED (B1)~~ **DONE in round 3** — canonical build reproduces the protected PIE/APB1 and a stable EFI digest | §9 |
| 5d | Formal regression (121 suites) | round 2: 22 pass / 99 blocked at the PIE guard. **Round 3 re-run on the canonical artifact** | §9.5 |
| 5c | ESP ↔ embedded-EFI identity | **DONE** | `env/`, `evidence.json` `image.boot_efi_sha256` |
| 6 | Gate command matches the claims | **DONE** — the gate now *fails* where the claims would be wrong | `tools/v1_gate.sh` + receipts + aggregate + `gate/gate-controls.txt` |
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
| `docs/v1/evidence/v2-2026-10-29-r2.tar.gz` | package | 111 files, 461 677 B, sha256 `07ff4d442871cd5c70b455b359544eaca754f6ba5a90928497cdd95808e08148`; verified from a fresh extraction, 0 problems |
| `…-r2/gate/receipts/*.json` | receipts | per-stage identity: commit, dirty paths, `code_fingerprint`, EFI/ESP digests, newest tracked source, exit code |
| `…-r2/gate/gate-controls.txt` | controls | the gate's own rejection tests (stale code fails; docs-only edit does not) |

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
6. **Formal regression is blocked, not green** (§5.1): 22 of 121 suites ran;
   99 could not, all at the PIE guard. Nothing in this round may be described
   as "all 121 suites pass".
7. **ESP bytes are not stable across rebuilds**: the FAT timestamps change
   with wall-clock time, so `build/arena-esp.img` hashes differently on each
   `tools/espimg.py` run while the embedded `BOOTX64.EFI` stays byte-identical
   to the EFI it was built from. Each stage receipt records the ESP it
   actually used; the ESP digest is therefore provenance, never an identity
   check, and a reader comparing an old digest with a newer file is seeing
   this effect rather than a changed artifact.
8. **Self-reference in the package receipts**: `gate/receipts/package.json`
   and `verify.json` were recorded by the packaging run, so the copy inside
   the archive names the digest of the archive produced just before the final
   repackaging. The authoritative pairing is the `.sha256` file beside the
   archive plus the fresh-extraction verify in §5.



---

## 9. Round 3 — official components recovered, B1 closed, canonical artifact

Round 3 began from C's review of `33d1915a`. The central correction: **the
official Rust target components were never missing.** They are tracked in
this repository at `docs/phase16/dependencies/rust-1.97.0/` as five binary
parts. Round 2's statement that they were "unavailable offline" was wrong,
and the dev-path substitute it justified was unnecessary.

### 9.1 Component recovery (executed, verified)

| Step | Command | Result |
|---|---|---|
| Parts verify | `sha256sum --check PARTS-SHA256SUMS` | 5/5 **OK** |
| Reassemble | `cat …part-* > …none.tar.xz` / `…uefi.tar.xz` | 14 907 620 B and 25 374 412 B |
| Archives verify | `sha256sum --check SHA256SUMS` | both **OK**: `5b6702d1…168a` (none), `2c48c8b6…1ad0` (uefi) |
| Install | `install.sh --prefix=/opt/rust/prefix --disable-ldconfig` | component `rust-std-x86_64-unknown-none` and `rust-std-x86_64-unknown-uefi` installed, exit 0 both |
| Provenance | archive `git-commit-hash` | `2d8144b7880597b6e6d3dfd63a9a9efae3f533d3` = the installed `rustc 1.97.0 (2d8144b78)` |

The reconstructed archives are **not** committed (README: do not duplicate);
they were built under `/tmp/rust-deps` and are reproducible from the tracked
parts.

Raw output: `recovery/rust-std-install.log`, `recovery/component-identity.txt`,
`recovery/component-resolution.txt`, `recovery/component-disambiguator.txt`.

Distinguishing official from source-built (file existence is not enough):

* the installed `libcore` for `x86_64-unknown-none` carries the crate
  disambiguator **`CsieveXhe8AWz_`** — the official value the protected
  fixture was built against. The source-built replacement round 2 installed
  carried `Csi1tY5tVYnWh_`, which is why round 2's digest could never match;
* `rustc --target <t> --print target-libdir` reports the private prefix, and
  a `#![no_std]` crate compiles for both targets with **no `-L` override**.

### 9.2 The second half of B1 — the documented build path

With the official components installed, the guard still refused, now with
`a65f3a1e…`. The cause was not the sysroot: `userspace/arena-platform/src/startup.rs`
embeds `file!()` in a panic string, and the committed fixture was built at
**`/workspace/ArenaOS`**, so building identical sources from
`/home/user/ArenaOS` embeds a different absolute path and changes the bytes.
Diffing the two ELFs showed the same size (23 256 B) and only shifted code
plus that path string.

Building from the documented path (`sudo mount --bind /home/user/ArenaOS
/workspace/ArenaOS`) reproduces the protected identities **exactly**:

| Identity | Expected (unchanged) | Reproduced |
|---|---|---|
| Phase-14 PIE | `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e` | **match** |
| Signed APB1 | `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa` | **match** |

No expected digest was edited, no guard weakened, and
`tools/build_phase16_dev.sh` was not used.

### 9.3 Canonical build result

`bash tools/build.sh --image` from `/workspace/ArenaOS`, exit **0**:

* EFI `bcd86866dbe7c15420eec958bff3024f66963d5d1dd39945513a58da750d5d83`
  (11 039 232 B) — **the same identity round 1 published**, i.e. an
  independent environment reproduced the canonical artifact;
* ESP `8a34f041…` with `/EFI/BOOT/BOOTX64.EFI` byte-identical to that EFI
  (`canonical-build/esp-embedded-efi.txt`);
* from-scratch rebuild (`rm -rf` all three target dirs, rebuild) reproduced
  the identical EFI digest (`canonical-build/rebuild-from-scratch.log`).

Environment: QEMU 11.1.2 patched `e0ea2b1f…` through isolated glibc 2.40
loader `63704534…`; OVMF code `7e34e3b8…`, vars `5d2ac383…`; rustc
`2d8144b78`; gcc 12.2.0; zig/clang 22.1.8.

### 9.4 Guest demonstrations on the canonical artifact

Both compiler cells pass every checkpoint, on EFI `bcd86866…`:

| Cell | Result |
|---|---|
| Guest proof, GCC | **PASS**, 8/8 checkpoints, frames published |
| Guest proof, Clang | **PASS**, 8/8 checkpoints, frames published |
| Refusal matrix, GCC | **PASS**, 21/21 cases |
| Refusal matrix, Clang | **PASS**, 21/21 cases |
| Lifecycle | **PASS**, 10/10 cycles |

A defect found and fixed here: checkpoint 7 asserted that each window
disappeared from a probe rectangle that a *second live window* could cover,
so the Clang cell failed while every Desktop receipt said the window was
gone. The probe now measures only the uncovered part and records an obscured
close as obscured, with the final all-closed frame required to show zero
app-coloured area.

### 9.5 Refusal matrix — what each case actually exercises

Round 2 published 21 guest cases as a single "refusal matrix". They do not
all exercise the same boundary, and the classification is now recorded per
case in the evidence (`classes`) and enforced: an unclassified case fails the
verdict.

| Class | Cases | What it proves |
|---|---|---|
| **IPC** | `wire-magic`, `wire-version`, `wire-opcode`, `wire-reserved`, `unexpected-transfer` | real frames on the real endpoint, refused by production dispatch |
| **KERNEL** | `layout-pages-minimum`, `layout-wrong-kind-refused`, `transfer-surface-retained` | real syscall/kernel fact: granted pages, wrong-kind refusal, retained capability |
| **DISCOVERY** | `reorder-reversed`, `reorder-rotated`, `missing/ambiguous/wrong-rights` × endpoint,surface | production discovery over **synthetic copies** of the descriptor |
| **IDENTITY** | `foreign-identity` | production identity validator over **mutated identity bytes** — not a real foreign capability |
| **CONTROL** | `baseline-record-accepted`, `identify-after-refusals`, `genuine-identity` | the genuine path still works |
| **CLIENT-ARITHMETIC** | `layout-pixel-window-fits` | **client-side comparison only** — see §9.6 |

Remaining gap, stated plainly: there is still no case that presents a *real*
foreign or stale capability (the launcher never grants one and no supported
fixture can manufacture one without new launch authority); `foreign-identity`
is a validator test over synthetic bytes.

### 9.6 Layout boundary — what is proven and what is not

Proven, in the guest: the granted surface region really carries
`GFX_TOTAL_PAGES` = **301** pages (production `SYS_SHARED_PAGES`), which is
exactly `4096 + 640×480×4` bytes; and the kernel refuses a region query on a
non-region capability.

**Not** proven in the guest: the *refusal* of a 300-page region. The check
that refuses it lives in the Rust client library
(`userspace/desktop/src/client.rs`: `bound[1]*4096 < PIXEL_OFFSET + w*h*4`),
which links into Rust applications; the C guest fixture cannot call it, and
the launcher does not grant a 300-page region to manufacture the case.
Changing `client.rs` to make it testable would change the kernel image and
therefore invalidate the canonical artifact qualified in §9.3, so it is
recorded as an open requirement rather than papered over with a host-side
transcription of the expression.

### 9.7 Qualification matrix (round 3)

| # | Requirement | State |
|---|---|---|
| 1 | Ack **and** visible response through production logic | **DONE** |
| 2 | Successful checkpoint frames published and validated | **DONE** |
| 3 | Cleanup compares all enforceable resources | **DONE** |
| 4 | Guest refusal matrix with precise classifications | **DONE**, one gap (§9.5) |
| 5 | Final-artifact stability, 100/100 | this artifact: see §9.8 |
| 5b | EFI rebuild reproducibility | **DONE** (§9.3) |
| 5c | ESP ↔ embedded EFI identity | **DONE** |
| 5d | Formal regression on the canonical artifact | see §9.8 |
| 5e | Layout page-count boundary, both sides, guest | **OPEN** (§9.6) |
| 6 | Gate command matches the claims | **DONE** |
| 7 | Review candidate + package without self-reference | **DONE** |

### 9.8 Outstanding

`docs/v1/evidence/v2-2026-10-29-r3/regression/run-tests.log` and the
stability loop on EFI `bcd86866…` are recorded in §10 when they finish.
