# Phase 13 progress

## Start record

- Starting documentation tip / branch parent: `330a691797343c8ef997cbb791ca54ec10e89fc5`
- Qualified Phase-12 implementation ancestor: `cd8c78a0189ce365fd5af93b0006f15fb647ed6f`
- Working branch: `arena/phase13-native-app-maturity`
- Starting tree: clean after checkout; implementation work began after the
  audit and environment receipt below.
- Read first: Phase-12 final report, Phase-13 handoff, compatibility handoff,
  Phase-12 plan, ADR-0080 through ADR-0089 and ADR-0090/0091.

## Current state

| Workstream | Status | Evidence / next action |
|---|---|---|
| 13.0 production-path audit | Complete | Read the required Phase-12 reports and ADRs; traced APB1 install, packaged policy, filesd roots, Desktop launch, Image registration, process teardown, scheduler mapping ownership, and resource bounds. Summary is in `PLAN.md`. |
| Toolchain and guest environment | Ready | Debian 13, unprivileged UID 1000, no sudo/root. Installed Rust 1.97.0 plus rustfmt/Clippy and `x86_64-unknown-none`/`x86_64-unknown-uefi` targets with official rustup under `/tmp`; extracted signed Debian snapshot QEMU 10.0.11 and OVMF 2025.02 under `/tmp`. `tools/dev-env/env.sh` and normal repository build tooling remain in use. |
| 13.1 installed registry and launch | Implemented; T1 guest proof passed | ADR-0092 trust split is wired through filesd, packaged, and Desktop. Boot rebuilds from protected installed APB1 state; launch re-verifies current receiver policy/tree and creates an exact bounded Image capability. See the T1 receipt below. |
| 13.2 launcher | Initial All Applications implementation; T1 guest proof passed | Registry-backed list, search, keyboard selection, pointer launch, and live-instance indication work for a real installed app. Persisted favorites and active-app dock composition remain. |
| 13.2 associations and Open With | Implemented; focused host and T1 guest proof passed | Signed content-type metadata filters handlers; user defaults persist in AFS2. Desktop offers only the selected File capability after an explicit choice. See the receipt below. |
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

## Phase-13 implementation receipts

### Installed registry, launch, and initial launcher T1

- Added protected installed-candidate enumerate/inspect/verify operations in
  filesd and a bounded descriptive catalog in packaged. The catalog is rebuilt
  from receiver-verified APB1 state at boot. Every launch rechecks current
  signer policy and the installed tree, transfers the verified executable into
  bounded staging, and asks the kernel Image authority to accept those bytes.
  App ID, package ID, version, and paths only select descriptive records; they
  do not authorize execution. APKG v1 policy remains separate.
- Added Desktop launch for the installed-app ABI-v2 profile. It receives a
  fresh Image capability from packaged and uses the existing exact startup
  capability inventory. The dynamic Image envelope is now 16 images of at most
  256 KiB each, with at most 128 load pages per image; ADR-0093 records the
  bound and ELF constraints.
- Added an All Applications surface backed by the verified catalog and the six
  built-in apps. Search, keyboard selection, pointer launch, and current-run
  indication are exercised. Pinned/favorite persistence and dock composition
  are still outstanding.
- `python3 tools/test_phase13_registry_guest.py` passed on QEMU 10.0.11 with
  OVMF 2025.02. The test builds and signs an APB1 package, seeds and installs
  it through the real guest path, opens the catalog, keyboard/search launches
  the real native ELF, then pointer-launches a second instance. Both processes
  audit ABI-v2 startup and publish real compositor windows. Closing both
  retires their ProcessGroups and returns identity-bearing resource counts to
  baseline; the guest shuts down cleanly.
- Measured `(free frames, process records, processes, SharedRegions, region
  pages, maps, caps)` at baseline: `(114307, 15, 15, 2, 470, 4, 45)`; with two
  installed instances: `(112309, 17, 17, 6, 2350, 9, 49)`; after close:
  `(114301, 15, 15, 2, 470, 4, 45)`. Process, region, map, and cap values
  returned exactly
  to baseline. Six additional frames were free after close; this is a net
  reclaim, not retained application memory. Other Phase-13 resource classes
  are not yet included in this fixture.
- Development fixes found by this proof: filesd now writes executable chunks
  at verified file offsets in the lent staging region; packaged and Desktop
  IPC argument ordering and strict zeroed syscall arguments were corrected;
  kind 6 now receives ABI-v2 bootstrap and close-event wakeups. The kernel's
  128-slot/slot-127 accounting behavior was preserved, and the APB1 install
  regression passed after the integration.
- Focused checks passed: arena-platform host tests (34/34), Desktop/filesd/
  packaged release builds and rustfmt checks, `test_m12_startup.py` (7/7 plus
  M1–M7 and M11 markers), and the real APB1 install/registry/launch guest above.
  This is a feature checkpoint; the full historical suite has not been run.

### Registry-backed associations and exact document handoff T1

- Desktop loads bounded signed association declarations from the current
  verified packaged catalog and keeps them separate from compact app identity
  records. The six built-in applications have explicit built-in declarations;
  installed associations come from the signed APB1 manifest.
- Files and the Desktop icon menu offer Open With. The Open With surface is the
  existing All Applications UI filtered to handlers for the inferred content
  type. `D` saves the selected handler ID in a canonical, checksummed
  `.arena-app-associations` record under `/Users/user` in AFS2. Defaults are
  pruned when the installed catalog no longer contains the selected handler.
  A default only chooses a descriptive registry record; it creates no process
  and grants no File capability.
- The exact selected File capability remains in the Desktop until handler
  selection. Cancel closes the chooser and releases it. Explicit Open With
  opens the source as a read-only filesd capability and launches the selected
  built-in or freshly reverified installed image through ABI v2. Filesd enforces
  the read-only grant even though the kernel service endpoint capability can
  invoke its read/write operation set. The startup ABI audit confirms that the
  selected app receives the expected File cap and no wider filesystem root.
- `python3 tools/test_phase13_registry_guest.py` passed on QEMU 10.0.11 / OVMF
  2025.02. The real Desktop menu opened the chooser; Escape returned process,
  region, map, and capability identity counts to the pre-request baseline. The
  test then searched for the signed installed handler and saved it as the
  `text/plain` default. Saving changed no identity-bearing count and did not
  launch an app. A second explicit selection launched the real installed ELF;
  the guest app read the exact `z-associated.txt` contents, verified write
  refusal, and the host confirmed the document bytes remained unchanged.
- In this run, boot identity counts were `(process records, processes,
  SharedRegions, region pages, maps, caps) = (15, 15, 2, 470, 4, 45)`. The
  canceled chooser returned to those exact counts. The document app ran at
  `(16, 16, 4, 1410, 8, 47)` while active and closed to `(15, 15, 2, 470, 4,
  45)`; four fewer free frames remained than the observed boot sample. The
  subsequent two-window launch/close sequence also returned identity counts to
  `(15, 15, 2, 470, 4, 45)`. The guest log is `build/serial-phase13-registry.log`.
- `cargo test --target x86_64-unknown-linux-gnu --lib` in Desktop passed
  82/82, including association codec, canonical corruption refusal, stale
  handler pruning, and explicit path-only Open With effects. Desktop and the
  Phase-13 application release builds passed; Python fixture compilation and
  `git diff --check` passed. The first guest attempt found the registry
  refresh's large stack-local table and led to moving bounded association
  arrays into static registry storage; the corrected guest boot is green.

This remains a feature checkpoint. Favorites/dock composition, multi-window
AppInstances, helper lifecycle, streams, process-wide VM, scalable heap, user
threads, synchronization, pressure, and final qualification remain open.
