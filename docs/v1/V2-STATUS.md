# V2 — V1 qualification status (durable)

**Owner:** V2 (replacement engineer). **Reviewer:** C. **Lead:** M.
**Last updated:** 2026-10-29.

This document is the single reconstruction point for the V1 qualification
task. Another reset should be recoverable from here plus the committed
scripts and evidence, without the conversation.

---

## 1. Source checkpoint and working branch

| Item | Value |
|---|---|
| Handoff checkpoint | `bfa4e85564a8e7840b862c7fdc89794190a34158` (verified with `git cat-file -t` → `commit`) |
| Handoff source branch | `arena/f6b7ebc4-arenaos` (merged into the checkpoint) |
| `main` at start | `d2bfd660040ddface1cd6e791301fb8ea73d181c`; local vs `origin/main` divergence `1 1` (checkpoint is one merge commit ahead) |
| Working branch used | `arena/af044828-arenaos` |
| Requested branch name | `arena/v2-v1-qualification` |

**Branch deviation, stated plainly.** The handoff asked for a separate
working branch such as `arena/v2-v1-qualification`. This session is bound to
`arena/af044828-arenaos`: the platform tracks the session by that branch and
forbids switching to, creating, or pushing any other. All work is therefore
committed to `arena/af044828-arenaos`, which was created from the handoff
checkpoint `bfa4e855` and shares its history. No force-push, no merge into
`main`, no rewriting of historical evidence.

**Build-path requirement (protected guard).** The Phase-14 static-PIE guard
in `tools/build_phase14_pie.sh` pins
`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`. The
fixture embeds its own absolute source path (a panic-location string), so the
hash only reproduces when the tree is built at the physical path
`/workspace/ArenaOS`. Building at `/home/user/ArenaOS` yields
`a65f3a1e9bc29158b509e5c267cbbacf53ae6ba0a39b2b5972f1cad56e086c05` and the
guard fails closed. Recovery is a bind mount:

```sh
sudo mkdir -p /workspace/ArenaOS
sudo mount --bind /home/user/ArenaOS /workspace/ArenaOS
cd /workspace/ArenaOS && source tools/dev-env/env.sh && bash tools/build.sh --image
```

**No build guard, expected digest, or protected hash was modified.** The
mount reproduces the accepted identity; the guard keeps failing on any other
path.

---

## 2. Accepted contract (unchanged by this work)

ADR-0114 is **accepted architecture**:
**Discover → Identify → validate an already-held surface → proceed.**

Boundaries maintained in this work:

- Discovery by kind + rights over the verified ARST v2 record; never slot or
  descriptor order. The trusted Desktop launcher profile mints exactly one
  `BadgedEndpoint` (kind 12, role Other, rights `WRITE|COPY|DESTROY` = 14)
  and exactly one writable session `SharedRegion` (kind 7, rights including
  `READ|WRITE|COPY`). Missing or ambiguous candidates are refused.
- Identify uses ADSK **version 2, opcode 13, fixed 64-byte frames**; bytes
  6–7 reserved zero in both directions; `b[8..10]` client protocol version 1
  in the request; reply `b[8..16]` session serial, `b[16]` surface kind,
  `b[17]` rights, `b[18..26]` surface object ID, `b[26..64]` reserved zero.
- Typed service status travels in the existing IPC reply status word
  (`SYS_IPC_CALL` `out[0]`), separately from the kernel syscall result. A
  successful reply requires both.
- `CAP_NONE` on both Identify request and reply; unexpected transferred
  capabilities are destroyed and the call refused.
- Desktop authenticates through the kernel-delivered badge and exactly one
  live session, and names only that session's surface.
- The client validates the full object ID, kind and rights against a
  capability it already holds (`SYS_CAP_DESCRIBE`).
- No client hidden-badge query, no new syscall, no weakened permission, no
  discovery-only fallback.
- ADSK-v1 is preserved verbatim for window operations.
- Surface layout preserved: 32-bit pixels, 4096-byte pixel offset, real
  bounds (`640x480x4` = 300 pages at page offset 1 → 301 pages).
- Primary failures preserved while cleanup failures are recorded; a required
  cleanup failure after an otherwise successful operation fails.
- Synchronous IPC has **no client-enforced deadline**; unresponsive-service
  behaviour relies on documented external supervision, never a client timer.

**Documentation staleness found and NOT acted on yet:** ADR-0114's header
still says "Implementation: pending", while the checkpoint already contains
the wire codec (`userspace/desktop/src/wire.rs`), the Desktop opcode-13
handler (`userspace/desktop/src/bin/desktop.rs`) and the client
implementation (`experiments/c-runtime/app/c_gfx_app.c`). This is a status
line to reconcile; the contract text itself needs no change.

---

## 3. Environment recovery

The reset left no toolchain, no QEMU, and no ESP. None of that was treated as
an environment blocker; each was rebuilt from committed instructions.

### 3.1 Tracked QEMU package (verified before any use)

| Check | Expected | Observed |
|---|---|---|
| Size | 95,562,916 | 95,562,916 |
| SHA-256 | `63b36649b4498d4e6be693a619ae3c04ff1fc53fdc62008d6ef9c3cbf4c01f73` | identical (matches committed sidecar `phase15-patched-tcg-qualification.tar.gz.sha256`) |
| Patched ELF | `e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff` | identical after extraction |
| Unpatched ELF | (package-recorded) | `443c81fc9482493297ae7a3f37300bef3a82a2873e428f72884cf52d16e27284` |

Archive contents restored under `/home/user/phase15-pkg`: `backend/
qemu-system-x86_64-{patched,unpatched}`, `backend/runtime/lib`
(GLib/Pixman/Slirp), `backend/pc-bios` (SeaBIOS + EDK2/OVMF), `firmware/
OVMF_CODE_4M.fd`, `firmware/OVMF_VARS_4M.fd`, `artifacts/` and `evidence/`.

### 3.2 Isolated runtime (host glibc 2.36 < required 2.38)

Host glibc is `2.36-9+deb12u4`, so the pinned binary cannot start directly.

```sh
bash tools/dev-env/build-isolated-glibc.sh    # m4, bison, gawk, glibc 2.40
```

Restricted toolchain, rebuilt from GitHub mirrors as the script documents
(`tar-mirror/gnu-m4`, `Distrotech/bison`, `gvlx/gawk`,
`bminor/glibc@9fe85766`), installed to `/home/user/buildtools` and
`/home/user/glibc-2.40`. The host system glibc is untouched. Wrappers at
`/home/user/phase16-backend/qemu-{patched,unpatched}` invoke the pinned ELF
through the private loader with explicit `-accel tcg -L <pkg>/backend/pc-bios`.

No symbol renaming, no version stripping, no preload shims: the wrapper only
chooses the loader and library path.

### 3.3 Rust 1.97.0 and the official bare-metal targets

`tools/dev-env/recover-rust-and-zig.sh` (new, committed) is the *compiler
recovery* half of `tools/dev-env/bootstrap.sh`, separated so a reset does not
re-run the older QEMU-npm / musl / source-built-sysroot stages.

- Rust from PyPI `arena-rust-toolchain[+data1..3]==1.97.0` → `/opt/rust/prefix`
- Official targets from the tracked dependency package
  `docs/phase16/dependencies/rust-1.97.0/`: all five parts pass
  `PARTS-SHA256SUMS`; the reassembled archives pass `SHA256SUMS`
  (`5b6702d1…168a` none, `2c48c8b6…1ad0` uefi); each installed with its own
  official `install.sh --prefix=/opt/rust/prefix`.

### 3.4 GCC and the supported Clang route

GCC is the host `gcc (Debian 12.2.0-14+deb12u1) 12.2.0`.

Clang is the repository's documented route: `experiments/c-runtime/run.py`
drives `zig cc -target x86_64-freestanding-none` and resolves the binary from
`ARENA_ZIG` (default `/opt/zig-clang/pkg/ziglang/zig`). Zig 0.17.0 is the
pinned PyPI `ziglang` wheel; `recover-rust-and-zig.sh` installs it to
`/opt/zig-clang`. Reported identity: `clang version 22.1.8`.

---

## 4. Verified completed work

Filled in as the work completes; each line carries its evidence location.

### 4.1 Environment

| Item | Result | Evidence |
|---|---|---|
| QEMU package integrity + extraction | PASS | `docs/v1/evidence/v2-2026-10-29/env/` |
| Pinned ELF identity | PASS `e0ea2b1f…f26ff` | same |
| Isolated glibc 2.40 build | see env receipt | `logs/isolated-glibc.log` in the env receipt |
| Emulator executes, TCG-only, `-accel tcg` | see env receipt | `qemu-version.txt`, `qemu-accel-help.txt` |
| Backend controls patched/unpatched | see `backend-controls/` | `tools/backend_controls.py` receipts |
| Rust 1.97.0 + official `none`/`uefi` targets compile | PASS | `rust-cargo-version.txt`, `rust-target-compile-*.txt` |
| GCC guest ELFs + ISA audit | PASS (`static_audit_pass: true`) | `compilers/` |
| Clang-via-Zig guest ELFs + ISA audit | PASS | `compilers/` |
| Protected guards unchanged | PASS (PIE `d7b0a8cf…`, APB1 `04add54f…`) | `guards/` |

---

## 5. Known defects (found by this review, not yet all closed)

1. **Global telemetry** — the previous harness combined `[gfx]` output
   globally (`last_gfx_events_count()`), discarding event type and instance
   identity. A counter increment from any instance satisfied any checkpoint.
2. **`baseline is None or …` escapes** — keyboard/mouse/isolation checks
   passed on a visible flash alone when no prior counter existed, so an
   acknowledgment was never actually required.
3. **Geometry as identity** — window rectangles (and connected components of
   app-colored pixels) were treated as instance identity, which is invalid
   as soon as windows overlap or merge.
4. **Historical settlement** — cleanup accepted a resource receipt from the
   pre-launch baseline rather than a fresh post-close one; per-instance
   retirement was counted globally.
5. **Negative controls with no substance** — some tested a copied policy or
   an empty serial slice, so they could not fail for the right reason.
6. **Provenance gaps that did not block PASS** — missing QEMU hash, missing
   compiler/ELF/APB1/EFI/ESP identities, and omitted artifacts were recorded
   but still produced an overall PASS.

Status of each closure is tracked in §6.

---

## 6. Remaining qualification matrix

Legend: **PASS** / **FAIL** / **NOT RUN** / **BLOCKED**, each with an
evidence reference. A checkpoint is only PASS with instance-attributed
acknowledgment *and* the stated visual/lifecycle evidence.

| # | Case | Status | Evidence |
|---|---|---|---|
| 1 | Launch-ready Desktop | NOT RUN | — |
| 2 | Protected-installer APB1 receipt | NOT RUN | — |
| 3 | Discover → Identify → Validate → native window | NOT RUN | — |
| 4 | Keyboard acknowledgment (type + instance) | NOT RUN | — |
| 5 | Mouse acknowledgment (type + instance) | NOT RUN | — |
| 6 | Two-instance focus/input isolation, both directions | NOT RUN | — |
| 7 | Retirements, exits, settlement vs baseline, visual removal | NOT RUN | — |
| 8 | Final demonstration | NOT RUN | — |
| 9 | GCC guest coverage | NOT RUN | — |
| 10 | Clang guest coverage | NOT RUN | — |
| 11 | Descriptor-reordering independence | NOT RUN | — |
| 12 | Missing/ambiguous capability refusal | NOT RUN | — |
| 13 | Malformed frame / unsupported version refusal | NOT RUN | — |
| 14 | Wrong-rights refusal | NOT RUN | — |
| 15 | Stale/foreign identity refusal | NOT RUN | — |
| 16 | Unexpected capability transfer refused + disposed | NOT RUN | — |
| 17 | Surface-layout boundaries | NOT RUN | — |
| 18 | Repeated launch/close (≥10 cycles) | NOT RUN | — |
| 19 | Bounded lifecycle and resource settlement | NOT RUN | — |
| 20 | Authoritative regressions | NOT RUN | — |
| 21 | Final-artifact stability gates | NOT RUN | — |

---

## 7. Evidence classification

| Class | Meaning |
|---|---|
| **GUEST** | Executed inside ArenaOS under QEMU TCG; serial/captures are the guest's own output |
| **HOST-ONLY** | Host unit/integration test through a mocked syscall boundary; says nothing about guest execution |
| **STATIC** | Checked from ELF/bytes without executing |
| **HISTORICAL (invalidated)** | Earlier run preserved for observation only; qualification verdict not valid |

Historical packages are preserved untouched and are **not** counted as
qualification evidence:

- `evidence/v1-guest-proof/runs/v1-proof-pass-2881fd6/` — the run named
  `v1-proof-pass-2881fd6` is **missing its serial log and QEMU stderr**.
  Those cannot be recreated as historical evidence and the verdict is
  invalidated (`docs/v1/REVIEW_CORRECTION_2881fd6.md`).
- `docs/v1/evidence-main-current-package/` — earlier development
  observations (main `d2bfd66` baseline boot).

---

## 8. Reconstruction quick start

```sh
# 1. QEMU package + isolated runtime (host glibc 2.36)
bash tools/dev-env/build-isolated-glibc.sh

# 2. Rust + Zig private toolchains
bash tools/dev-env/recover-rust-and-zig.sh

# 3. Protected build path (Phase-14 PIE guard depends on it)
sudo mkdir -p /workspace/ArenaOS && sudo mount --bind "$PWD" /workspace/ArenaOS
cd /workspace/ArenaOS && source tools/dev-env/env.sh && bash tools/build.sh --image

# 4. Harness controls (no QEMU)
python3 tools/v1_guest_proof.py --controls

# 5. Guest qualification
ARENA_ZIG=/opt/zig-clang/ziglang/zig python3 tools/v1_guest_proof.py
```
