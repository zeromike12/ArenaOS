# V2 — V1 qualification status (durable)

**Owner:** V2 (replacement engineer). **Lead:** M. **Reviewer:** C.
**Last updated:** 2026-10-29.

This document is the single reconstruction point for V1 qualification. It is
written so that a reader with only this repository — no conversation history —
can rebuild the environment, re-run every gate, and decide for themselves
whether the evidence supports the verdict.

---

## 1. Source checkpoint and working branch

| Item | Value |
|---|---|
| Handoff checkpoint | `bfa4e85564a8e7840b862c7fdc89794190a34158` (verified with `git cat-file -t`) |
| Working branch | `arena/af044828-arenaos` |
| Requested branch | `arena/v2-v1-qualification` — **not used**, see below |
| `main` at handoff | `d2bfd660040ddface1cd6e791301fb8ea73d181c`; checkpoint is 1 ahead / 0 behind |

**Branch deviation.** The handoff asked for a fresh working branch. This
session is bound to `arena/af044828-arenaos`: the platform tracks the session
by that branch and forbids creating, switching to, or pushing any other. All
work is committed there, branched from the handoff checkpoint. Nothing is
force-pushed, nothing is merged into `main`, and no historical evidence was
rewritten.

**Build-path requirement (protected guard, unchanged).** The Phase-14 static
PIE guard pins `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`.
The fixture embeds its own absolute source path, so the digest reproduces
only when the tree is built at the physical path `/workspace/ArenaOS`.
Building at `/home/user/ArenaOS` yields
`a65f3a1e9bc29158b509e5c267cbbacf53ae6ba0a39b2b5972f1cad56e086c05` and the
guard fails closed. Recovery is a bind mount:

```sh
sudo mkdir -p /workspace/ArenaOS
sudo mount --bind /home/user/ArenaOS /workspace/ArenaOS
cd /workspace/ArenaOS && source tools/dev-env/env.sh && bash tools/build.sh --image
```

No build guard, expected digest or protected hash was modified.

---

## 2. Accepted contract (unchanged by this work)

ADR-0114 is **accepted**: Discover → Identify → validate the already-held
surface → proceed. Boundaries preserved in this work:

- Discovery only through the verified ARST v2 startup record, by kind and
  rights; never by slot index or descriptor order.
- Exactly one conforming endpoint and one conforming surface, else refuse
  (codes 2/3/4/5/6). The trusted Desktop launcher profile is the only writer.
- ADSK v2 Identify, opcode 13, fixed 64-byte frames, bytes 6–7 reserved zero.
- Service status travels in the existing IPC reply status word, separate from
  the kernel syscall result.
- `CAP_NONE` on both the Identify request and its reply; the Desktop
  authenticates the caller from the kernel-delivered badge and names that
  session's surface.
- The client validates the full object ID, kind and required rights against
  the authority it actually holds (`describe`), and refuses a foreign or
  stale object.
- ADSK-v1 compatibility, the 32-bit surface layout, the 4096-byte pixel
  offset and the real bounds are all unchanged.
- Primary failures are preserved and cleanup failures recorded; a required
  cleanup failure after an otherwise successful operation still fails.
- Sync IPC has no client deadline by design; unresponsive-service behaviour is
  covered by documented external supervision, not by inventing a timeout.

Nothing in this work changed the contract, the wire format, the syscall
surface or any permission.

---

## 3. Environment recovery

Host: Debian 12, glibc 2.36, no KVM (TCG is the required accelerator).

### 3.1 Tracked QEMU package

```sh
sha256sum /home/user/phase15-pkg/backend/qemu-system-x86_64-patched
# e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff
sha256sum /home/user/phase15-pkg/backend/qemu-system-x86_64-unpatched
# 443c81fc9482493297ae7a3f37300bef3a82a2873e428f72884cf52d16e27284
```

Packaged tarball `63b36649…f73` (95,562,916 B) matches its sidecar. The
patched ELF is what every qualification boot uses; the unpatched ELF is kept
only as a negative control (`tools/backend_controls.py`).

### 3.2 Isolated runtime (host glibc 2.36 < required 2.38)

`tools/dev-env/build-isolated-glibc.sh` builds glibc 2.40 into
`/home/user/glibc-2.40` and wraps the emulator with the matching loader.
Observed identities:

| Object | SHA-256 |
|---|---|
| `libc.so.6` | `eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f` |
| `ld-linux-x86-64.so.2` | `63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458` |
| `qemu-patched` wrapper | `92ec1c12cc174d549dfdbedf921984784252d66630bdeca32d248085b42c76a1` |

`$ARENA_QEMU --version` → QEMU emulator version 11.1.2; `-accel help` lists
exactly `tcg`. Backend controls: patched **PASS 16/16** (exit 33), unpatched
**FAIL** (exit 35) — the negative control behaves as designed.

### 3.3 Rust 1.97.0 and the official bare-metal targets

Installed into a private prefix (`/opt/rust/prefix`) from the official
component tarballs, then both targets were *compiled against*, not merely
listed:

- `rustc 1.97.0 (2d8144b78 2026-07-07)`, `cargo 1.97.0 (c980f4866 2026-06-30)`
- `x86_64-unknown-none` → `rustc --crate-type staticlib` exit 0, 10.5 MB archive
- `x86_64-unknown-uefi` → compile exit 0
  (`env/rust-target-compile-*.txt`)

### 3.4 GCC and the supported Clang route

`gcc (Debian 12.2.0-14+deb12u1) 12.2.0`;
`clang version 22.1.8` via Zig 0.17.0 (`/opt/zig-clang`). Both build the
native guest application and both pass the ISA audit
(`run.py build --cc gcc --cc clang`, `run.py audit`: 10 ELFs clean).

---

## 4. Verified completed work

Every row below is a run that happened, with the artifact it produced.

| Item | Result | Evidence |
|---|---|---|
| QEMU package integrity, extraction, pinned ELF | PASS `e0ea2b1f…f26ff` | `env/qemu-package*.txt`, `env/qemu-identities.txt` |
| Emulator under the isolated loader, TCG only | PASS, 11.1.2, `-accel tcg` | `env/qemu-version.txt`, `env/qemu-accel-help.txt` |
| Backend controls patched / unpatched | PASS 16/16 vs FAIL | `env/backend-controls/xm-verify.json` |
| Rust 1.97.0 + official `none`/`uefi` targets | PASS (compiled) | `env/rust-cargo-version.txt`, `env/rust-target-compile-*.txt` |
| GCC and Clang guest toolchains + ISA audit | PASS | `env/compilers/` |
| Protected guards unchanged (PIE `d7b0a8cf…`, APB1) | PASS | `env/guards/` |
| Guest proof, GCC cell — 8 checkpoints | **PASS** | `guest-gcc/evidence.json` |
| Guest proof, Clang cell — 8 checkpoints | **PASS** | `guest-clang/evidence.json` |
| Ten launch/close cycles | **10/10 PASS** | `lifecycle-cycles/cycles-evidence.json` |
| Harness focused controls | **19/19 PASS** | gate log stage 2 |
| Repository regression suite | **ALL TESTS PASSED (121 suites)** | `regressions/run-tests.log` |

Run provenance for the published package:

| Fact | Value |
|---|---|
| Gate source commit | `3d340622` (`working_tree_dirty: false`, no other changed paths) |
| Boot image the guest ran | `arena-boot.efi` `bcd86866dbe7…` — identical on the ESP and in `build/`, image newer than every tracked source file |
| GCC client ELF / APB1 | `07f9fbc5d7dc…` / `073f08f79f3b…` |
| Clang client ELF / APB1 | `30cab76f2c94…` / `c0da952974c6…` |
| Archive | `v2-2026-10-29.tar.gz` `ba83c3bab766…6fc`, 97 files, verified from a fresh extraction and from a fresh `git clone` |

Guest-proven facts from the serial evidence (GCC cell; the Clang cell repeats
them with a differently-built client):

- `[gfx] identify ok surface=159 → [desktop] ordinary window placed handle=1
  surface=159 x=70 y=60 w=640 h=480` → rendered frame.
- `[gfx] event type=1 events=3 surface=159` (keyboard) and `[gfx] event type=2
  events=4 surface=159` (pointer), each with the flash visible inside that
  instance's own rectangle.
- Two concurrent instances (159, 164): focus acknowledged in both directions,
  per-instance key acks `9→10` and `4→5`, zero cross-instance acks.
- `[desktop] application retired: kind=6 surface=159 …` and `[desktop] child
  Process-cap exit surface=159 status=0`, then a fresh resource-detail line
  back to `app-instances=0 windows=0`.
- Cycles: surfaces 159, 164, 169 … 204 — a new object each time, each retired
  with status 0 and settlement to the pre-cycle baseline.

### 4.1 A defect this section caught in passing

The first gate reported PASS while its evidence recorded an EFI digest that no
longer matched a rebuild at the same commit: the harness had booted an image
built before a formatting-only change. Evidence now records the boot image's
age against the newest tracked source file and the EFI digest from both the
ESP and `build/`, and the verdict fails when they disagree — see §5.

## 5. Known defects and their status

| # | Defect found in review | Status |
|---|---|---|
| 1 | Telemetry merged globally; event type and instance discarded | **Closed** — `parse_gfx_events` keeps type + surface; `evaluate_event_ack` requires the named instance, the named type and a counter advance for *that* instance |
| 2 | `baseline is None or …` let a visible flash pass without an ack | **Closed** — missing telemetry fails; a flash alone never satisfies a checkpoint |
| 3 | Screen geometry used as instance identity | **Closed** — geometry comes from the Desktop's placement receipt and is still confirmed by an acknowledgment from that instance |
| 4 | Settlement accepted historical snapshots / incomplete accounting | **Closed** — `evaluate_cleanup` uses only lines after the close cursor, requires the *latest* detail line to match the pre-launch baseline, and rejects spurious retirements |
| 5 | Negative controls tested a copied policy or an empty slice | **Closed** — 19 controls drive the real evaluation functions; each fails for one specific reason |
| 6 | Missing provenance did not block PASS | **Closed** — a single `compute_overall_verdict` requires every checkpoint PASS, the serial log, the harness transcript, and every identity (commit, compilers, ELF, APB1, ESP, EFI, emulator ELF, loader, firmware) |

| 7 | The harness could boot a stale image and still PASS | **Closed** — verdict fails when the boot image is older than the newest tracked source file, or when the EFI embedded in the ESP differs from `build/arena-boot.efi` |
| 8 | Evidence could be copied into the package by hand after the fact | **Closed** — `V1_EVIDENCE_DIR` makes the run write where the verdict says the artifacts are; packaging refuses a single artifact over 32 MB so scratch state cannot masquerade as evidence |

Two production-side gaps were found while proving the above and were fixed
through the production path, not the harness:

- The Desktop logged a placement line on only one of its two `Create` paths.
  The second instance *adopts* a pre-created console window, so the receipt
  was missing exactly where it was needed. `log_placement()` is now called on
  both paths (`userspace/desktop/src/bin/desktop.rs`).
- The native client animated for 300 frames (~10 s), which is shorter than an
  interactive qualification pass (focus, keyboard, pointer, close). It now
  runs 1200 frames (~40 s) — still strictly bounded, so the lifecycle is
  finite and the teardown receipts remain the proof of a clean exit.

---

## 6. Qualification matrix

Legend: **GUEST** = executed inside ArenaOS under QEMU TCG (serial, screenshots
and Desktop receipts are the guest's own output); **HOST-ONLY** = exercised
through a mocked syscall boundary on the host; **STATIC** = computed from
bytes without executing.

| # | Case | Verdict | Class | Evidence |
|---|---|---|---|---|
| 1 | Launch-ready Desktop | PASS | GUEST | `guest-gcc/shots/cp01-launch-ready-desktop.png`, serial `real desktop frame presented` |
| 2 | Protected-installer APB1 receipt | PASS | GUEST | `guest-gcc/install-ack.json` (signed version 42) |
| 3 | Discover → Identify → Validate → native window | PASS | GUEST | `guest-gcc/evidence.json` cp3, `shots/cp03-first-native-c-window.png` |
| 4 | Keyboard acknowledgment (type + instance) | PASS | GUEST | cp4, `shots/cp04-keyboard-response.png` |
| 5 | Mouse acknowledgment (type + instance) | PASS | GUEST | cp5, `shots/cp05-mouse-response.png` |
| 6 | Two-instance focus/input isolation, both directions | PASS | GUEST | cp6, `shots/cp06-focus-isolation.png` |
| 7 | Retirements, exits, settlement vs baseline, visual removal | PASS | GUEST | cp7, `shots/cp07-desktop-restored.png` |
| 8 | Final demonstration | PASS | GUEST | cp8, `shots/cp08-final-demonstration.png` |
| 9 | GCC guest coverage | PASS | GUEST | `guest-gcc/` |
| 10 | Clang guest coverage | PASS | GUEST | `guest-clang/` |
| 11 | Descriptor-reordering independence | PASS | HOST-ONLY | `test_gfx_discover_permutation_independence` (reversed, rotated, relocated slots) |
| 12 | Missing/ambiguous capability refusal | PASS | HOST-ONLY | `test_gfx_discover_refusals` (codes 2/3/4/5/6) |
| 13 | Malformed frame / unsupported version refusal | PASS | HOST-ONLY | `test_gfx_identify_{wrong_magic,wrong_version,wrong_opcode,reserved_nonzero,wrong_kind,wrong_rights}`; desktop `wire::identify_tests` |
| 14 | Wrong-rights refusal | PASS | HOST-ONLY | `test_gfx_discover_refusals`, `test_gfx_identify_wrong_rights`, `test_gfx_identify_describe_rights_insufficient` |
| 15 | Stale/foreign identity refusal | PASS | HOST-ONLY | `test_gfx_identify_describe_{object_id_mismatch,kind_mismatch}`, `test_gfx_identify_service_error` |
| 16 | Unexpected capability transfer refused + disposed | PASS | HOST-ONLY | `test_gfx_identify_unexpected_landed_cap` |
| 17 | Surface-layout boundaries | PASS | HOST-ONLY | `test_gfx_surface_layout_boundary` (page 4096, offset 4096, 640×480×4, 300 pixel pages, 301 minimum, 32-bit, no padding) |
| 18 | Repeated launch/close (≥10 cycles) | PASS | GUEST | `lifecycle-cycles/cycles-evidence.json` — 10/10, surfaces 159…204 |
| 19 | Bounded lifecycle and resource settlement | PASS | GUEST | per-cycle fresh detail line back to `app-instances=0 windows=0`; `[gfx] exit events=… focused=… code=0` |
| 20 | Authoritative regressions | **PASS** (121 suites) | GUEST+HOST | `regressions/run-tests.log` |
| 21 | Final-artifact stability gates | **PASS** | STATIC | §6.2; `env/guards/`, gate stage 0 and stage 6 |

### 6.1 Authoritative regressions

`bash tools/run_tests.sh` at source commit `bdf6c66` with a clean tree:

```
QUALIFICATION SOURCE COMMIT: bdf6c66971574f63687f23bbd69f08415d5c37f4
QUALIFICATION SOURCE CLEAN: yes
...
QUALIFICATION SOURCE END: bdf6c66971574f63687f23bbd69f08415d5c37f4
ALL TESTS PASSED (121 test suites)
```

The transcript is committed unmodified as `regressions/run-tests.log`. The
runner re-checks at the end that HEAD and the working-tree status are what
they were at the start, so a PASS cannot hide a source change made mid-run.
Negative controls inside the suite are expected to print `FAILED -> RED PASS`;
those are fixtures proving that the production oracles can fail.

### 6.2 Final-artifact stability

- Phase-14 static-PIE digest `d7b0a8cf…`: unchanged, and still fails closed at
  any other build path (`env/guards/`).
- APB1 protected digest: unchanged.
- Kernel ISA audit: PASS (120,911 instructions, six trusted leaf profiles).
- `arena-boot.efi` is byte-identical across consecutive builds of the same
  tree (`bcd86866…`), and the gate builds before it boots.
- **The ESP image is not byte-stable, by construction:** `pyfatfs` writes FAT
  directory timestamps, so `arena-esp.img` differs between builds even when
  its contents do not. The evidence therefore pins the *EFI extracted from the
  ESP* and compares it to `build/arena-boot.efi`, rather than pretending the
  FAT image is reproducible.
- The evidence archive is re-verified from a fresh extraction outside the
  workspace (`97 files, problems: 0`), and separately from a fresh `git clone`.

### 6.2 Final-artifact stability

- Phase-14 static-PIE digest: `d7b0a8cf…` (guard unchanged, reproduces only at
  the pinned path).
- APB1 protected digest: unchanged.
- Kernel ISA audit: PASS (120,911 instructions, six trusted leaf profiles).
- The evidence archive is re-verified from a fresh extraction outside the
  workspace by `tools/v1_evidence.py verify`, which re-computes every digest
  and rejects unlisted files.

---

## 7. Evidence classification and locations

| Path | Class | Contents |
|---|---|---|
| `docs/v1/evidence/v2-2026-10-29/env/` | GUEST + STATIC | emulator/loader/firmware identities, TCG proof, backend controls, Rust/GCC/Clang receipts, guard digests |
| `docs/v1/evidence/v2-2026-10-29/guest-gcc/` | GUEST | serial log, harness transcript, `evidence.json`, install ack, 8 screenshots, QEMU stderr |
| `docs/v1/evidence/v2-2026-10-29/guest-clang/` | GUEST | the same, with the Clang-built client |
| `docs/v1/evidence/v2-2026-10-29/lifecycle-cycles/` | GUEST | 10-cycle verdict, serial log, per-cycle screenshots |
| `docs/v1/evidence/v2-2026-10-29/regressions/` | GUEST+HOST | `run_tests.sh` transcript, unmodified, with the runner's own commit assertions |
| `docs/v1/evidence/v2-2026-10-29.tar.gz` + `.tar.gz.sha256` | — | the durable package (manifest, README, archive) |

Screenshots are stored as PNG. QEMU writes PPM; packaging converts them
losslessly (no resize, no recolour) and says so in the package README, because
1.44 MB per frame is not worth keeping in a repository.

**Historical evidence, preserved and not counted.** `evidence/v1-guest-proof/runs/v1-proof-pass-2881fd6/`
is kept untouched. It is missing its serial log and QEMU stderr, so its PASS
cannot be re-verified and it is **not** used for any verdict here
(`docs/v1/REVIEW_CORRECTION_2881fd6.md`). The run in §4 is a new, clearly
labelled run of the corrected harness; it does not replace the old one.

**Provenance note.** `evidence.json` in each cell records the commit the run
executed against (`3d340622`) and `working_tree_dirty: false` with an empty
list of other changed paths. The evidence files themselves were committed
afterwards (`82ef9a3`). Nothing in the evidence was written by hand, and
`V1_EVIDENCE_DIR` means the harness wrote them where they now sit.

**Reproduce:**

```sh
bash tools/v1_gate.sh          # build → env receipts → controls → GCC → Clang → cycles → package+verify
python3 tools/v1_evidence.py verify docs/v1/evidence/v2-2026-10-29.tar.gz
```

---

## 8. Known limitations (stated, not hidden)

1. Cases 11–17 are HOST-ONLY. They drive the production `discover()`,
   `identify_exchange()` and the production Desktop wire codecs against
   synthetic records and a mocked syscall boundary. They prove the *decisions*
   are right and that the code is reachable, but they do not prove guest
   execution of each refusal: a guest-side negative client per refusal is
   still outstanding work. No verdict above claims otherwise.
2. The unresponsive-service case is not exercised inside the guest: sync IPC
   has no client deadline by design, and the documented supervision path is
   external. No test claims to prove a guest-side timeout.
3. The emulator runs under QEMU TCG, so frame timing is not representative of
   hardware; the proof is about correctness of delivery and attribution, not
   performance.
4. The bind mount at `/workspace/ArenaOS` is required for the protected PIE
   digest. On a machine where that path cannot be created, the build guard
   fails closed — by design, and this document does not work around it.
