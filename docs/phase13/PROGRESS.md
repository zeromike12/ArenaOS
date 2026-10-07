# Phase 13 progress

## Start record

- Starting documentation tip / branch parent: `330a691797343c8ef997cbb791ca54ec10e89fc5`
- Qualified Phase-12 implementation ancestor: `cd8c78a0189ce365fd5af93b0006f15fb647ed6f`
- Working branch: `arena/phase13-native-app-maturity`
- Starting tree: clean after checkout; no Phase-13 source edits have been made.
- Read first: Phase-12 final report, Phase-13 handoff, compatibility handoff,
  Phase-12 plan, ADR-0080 through ADR-0089 and ADR-0090/0091.

## Current state

| Workstream | Status | Evidence / next action |
|---|---|---|
| 13.0 production-path audit | Complete | Read the required Phase-12 reports and ADRs; traced APB1 install, packaged policy, filesd roots, Desktop launch, Image registration, process teardown, scheduler mapping ownership, and resource bounds. Summary is in `PLAN.md`. |
| Toolchain and guest environment | Ready | Debian 13, unprivileged UID 1000, no sudo/root. Installed Rust 1.97.0 plus rustfmt/Clippy and `x86_64-unknown-none`/`x86_64-unknown-uefi` targets with official rustup under `/tmp`; extracted signed Debian snapshot QEMU 10.0.11 and OVMF 2025.02 under `/tmp`. `tools/dev-env/env.sh` and normal repository build tooling remain in use. |
| 13.1 installed registry and launch | Design accepted; implementation in progress | ADR-0092 assigns protected AFS2 enumeration/reverification to filesd, current signer policy and the descriptive catalog to packaged, and exact Image-cap launch authority to the existing ABI-v2 broker path. Next: add narrow filesd scan/verify operations, wire boot rebuild, and implement real installed ELF resolution/launch. |
| 13.2 launcher and associations | Not started | Desktop accepts only built-in kinds 0–5; association/default model is descriptive host-side logic and is not persisted or used by Desktop. |
| 13.3 windows, helpers, lifecycle | Not started | Production session couples one Process child to one ordinary window. `AppInstanceTable`, `ProcessGroup`, and `WindowSet` are foundations; group/helper and multi-window production policy is absent. |
| 13.4 streams | Not started | ABI-v2 reserves stream roles; no native stream object or endpoint exists. |
| 13.5 VM and heap | Not started | 32-page heap uses writable/NX owned frames through slot 63; mapping tracking is per scheduler thread and ordinary map release/protection is absent. |
| 13.6 user threads and synchronization | Not started | Scheduler supports kernel-managed threads, but no ring-3 thread creation ABI exists. FS.base is saved per scheduler thread; mapping validation is per thread. |
| 13.7 pressure and PIE | Not started | Existing ELF validator is static ET_EXEC-only; dynamic Image registry is 2 entries × 4 KiB. Resource pressure and ASLR scope need an ADR and guest evidence. |
| Final qualification | Not started | No source implementation or guest evidence yet. |

## Baseline evidence before implementation

- `tools/build.sh --image`: passed with Rust 1.97.0; the kernel EFI was
  6,360,576 bytes at the build step and the 8 MiB ESP was created. The
  kernel's no-FPU/SSE/MMX image audit passed. The first ESP attempt exposed a
  missing Python `pkg_resources` shim in the newest setuptools; using the
  repository-documented pyfatfs dependency with setuptools 81 fixed the local
  tool bootstrap without changing repository files.
- `python3 tools/test_m12_startup.py`: passed on a real QEMU 10.0.11 / OVMF
  2025.02 guest, 7/7 M12 checks plus M1–M7 and M11 regression markers, and
  clean shutdown. The first QEMU process exited before firmware because the
  unprivileged Debian extraction needed SeaBIOS ROM files merged into its
  temporary QEMU data path; the stderr identified the missing ROM and the
  corrected wrapper then passed. This was an environment correction, not an
  accepted guest result.
- No historical full-suite run has been made during the initial audit.

## Audit facts

- APB1 is separate from APKG v1. `filesd` owns the protected application and
  staging roots, re-verifies the signature and durable installed tree, and
  publishes by atomic AFS2 activation. `packaged` owns signer policy and
  forwards the narrow slot-127 install authority.
- The catalog and installed-tree scanner already verify current signer
  eligibility and exact file-tree contents, but are only a pure library path;
  no trusted boot owner uses them to publish a production registry.
- App ID, package ID, version, names, and installed path are descriptive.
  Kernel execution authority is an Image capability. The current dynamic Image
  table accepts at most 4 KiB per image and has two live entries.
- Desktop production launch still selects one of six built-in kinds. It
  reserves one session row, one surface/snapshot set, one process group child,
  and one window per launch. The file-open path recognizes Editor kind 2.
- Process owns a PML4 and cap table. Scheduler thread records own user pointer
  regions and FS.base. `SYS_MAP_MEMORY` maps RAM as writable/NX and MMIO as
  uncached/NX, while address-space destruction is the ordinary memory release
  path. Any true user-thread design depends on first moving mapping validation
  to process-owned state.
- `packaged` owns the receiver-verified APKG policy chain and `ImageRegistrar`
  grant. It already re-verifies and launches APKG v1 payloads through the
  volatile kernel Image mechanism. That mechanism is distinct from APB1's
  protected AFS2 install record: it currently allows only two live dynamic
  Images, each at most 4 KiB, and at most four dynamic-image child processes.
  Desktop receives the packaged service endpoint but does not own the
  registrar or APB1 signer policy.
- The required trust split is therefore: the package-policy owner resolves a
  descriptive APB1 application to a current eligible installed version and
  asks filesd to re-verify the exact installed tree; only then may it produce
  a transient exact Image capability for Desktop's normal ABI-v2 spawn path.
  A registry entry itself remains metadata and cannot be passed to `SYS_SPAWN`.

## Environment receipt

| Tool | Version / location |
|---|---|
| Rust compiler | `rustc 1.97.0 (2d8144b78 2026-07-07)` from official rustup |
| Cargo | `cargo 1.97.0 (c980f4866 2026-06-30)` |
| Rustfmt | `rustfmt 1.9.0-stable (2d8144b788 2026-07-07)` |
| Rust targets | `x86_64-unknown-none`, `x86_64-unknown-uefi` |
| QEMU | `10.0.11` from signed Debian trixie snapshot package `1:10.0.11+ds-0+deb13u1` |
| OVMF | `2025.02-8+deb13u1`, 4 MiB CODE/VARS pair from the same Debian snapshot |
| Privilege model | UID 1000, no root/sudo; all installed tool files are under `/tmp` |

## Checkpoint discipline

No resource constants are increased until the corresponding implementation
and admission inventory are understood. Each coherent feature will get a
targeted host/model proof and 1–3 real guest boots; architectural scheduler,
address-space, or teardown changes get broader T3 evidence before preservation
checkpoints. The full historical suite and 100-boot witness are reserved for
the source-frozen final qualification.
