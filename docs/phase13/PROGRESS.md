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
| 13.3 windows, helpers, lifecycle | Multi-window/headless launch, per-instance groups, and stable process exit status implemented; signed helper execution remains | ADR-0097 gives each additional window its own SharedRegion, snapshot, compositor and publication state under one authenticated process session. ADR-0098 adds signed headless Image launch with explicit caps and Process-cap reap. ADR-0099 gives every live Desktop app instance its own four-member `ProcessGroup`, with exact group teardown. ADR-0100 adds stable final status through exact Process/READ authority. Guest proves three windows per process, two concurrent windowed instances, one windowless headless instance with status 42, and 32-session group-slot reuse; signed helper allowlisting remains. |
| 13.4 streams | Not started | ABI-v2 reserves stream roles; no native stream object or endpoint exists. |
| 13.5 VM and heap | Implemented; T1 and targeted historical regressions passed; T3 preservation passed | ADR-0095 adds exact-cap process VM reserve/commit/protect/release/query, guard pages, zeroed lazy backing, W^X, and kernel accounting. ADR-0096 adds a lazy 16 MiB ScalableHeap while preserving the Phase-12 32-page BoundedHeap. The prior implementation checkpoint is preserved at `7097eb5`; later Phase-13 work continues on this branch. |
| 13.6 user threads and synchronization | Not started | Scheduler supports kernel-managed threads, but no ring-3 thread creation ABI exists. FS.base and kernel execution state are saved per scheduler thread; syscall mapping validation is shared through Process. |
| 13.7 pressure and PIE | Not started | Existing ELF validator is static ET_EXEC-only; dynamic Image registry is 2 entries × 4 KiB. Resource pressure and ASLR scope need an ADR and guest evidence. |
| Final qualification | Not started | No source freeze, complete historical suite, 100-boot receipt, or extracted-archive witness yet. |

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

At this registry/association checkpoint, favorites/dock composition,
multi-window ownership, helper lifecycle, streams, user threads,
synchronization, pressure, and final qualification were still open; later
subsections record the subsequent VM/heap and multi-window checkpoints.

The current branch has since added multi-window ownership (ADR-0097), and the
signed registry guest now also installs and launches a headless package as
described in ADR-0098. The earlier checkpoint statement is retained as a dated
status record; helper lifecycle, streams, user threads, synchronization,
pressure, favorites, and final qualification remain open.

### Process-owned user mapping inventory implementation

- Added the unchanged 80-entry page-span inventory to each `Process` and
  removed that table from every scheduler thread. The bounded 80-entry
  scheduler-local table remains only for kernel-owned ring-3 self-tests that
  have no Process object; it is one shared table, not 80 entries per kernel
  thread.
- `current_user_regions`, initial image/stack registration, append, and exact
  removal now resolve to the current Process for every production ring-3
  thread. Syscall buffer validation and existing owned-frame/SharedRegion
  mapping paths therefore share address-space metadata. Process teardown
  discards the inventory with its Process record and continues to reclaim PTEs
  and SharedRegion pins through the existing teardown owners.
- No resource limit or syscall number changed. The fixed range-record array
  storage is moved from the 64 scheduler slots to the 64 process slots; the
  80-span per-owner budget is unchanged. This is the first half of 13.8 only:
  native reservations, lazy commitment, private release/protection, guard
  ranges, and mapping accounting still need implementation.
- Added ADR-0094 to record the ownership change and its single-core atomicity
  condition. Kernel target check passed. `test_m12_startup.py` passed 7/7 M12
  checks plus M1–M7 and M11; `test_m12_scale.py` passed 32 live sessions,
  mutation-free refusal, 16-close/16-reuse, and full teardown; the Phase-13
  installed registry/association guest passed after this kernel change.
- Rebuilt the exact EFI (`SHA-256 256a03461bf1eb6f2c85595f9ff347fd42dd7272e7328d7b1f27088a02bbf515`)
  and ran `tools/stability_loop.sh 20`: 20/20 fresh boots passed, zero
  failures, in 149 seconds. The 32-session run observed 77 retained page-table
  frames after all app sessions closed; processes, regions, pages, maps, and
  caps returned to the documented baseline.

### Process-owned VM and scalable heap T1

- ADR-0095 adds a process-owned VM registry. Exact non-copyable VmRegion caps
  authorize guarded reserve, zeroed lazy commit, read-only/read-execute/read-
  write protect, exact release, and query. Admission is bounded at 128 regions
  globally, eight per process, 4,096 pages per region, 8,192 committed pages
  per process, 32,768 globally, and 64 pages per commit/protect operation.
  Region records are descriptive; cap possession authorizes every operation.
- The process-owned 80-span validation table is shared by all its threads.
  Pointer checks require a present USER PTE for every touched page and WRITE
  on syscall output buffers. This makes uncommitted pages, guards, read-only
  pages, read-execute pages, MMIO, and released ranges fail closed as syscall
  buffers.
- ADR-0096 adds ScalableHeap alongside the unchanged Phase-12 BoundedHeap.
  Each Phase-13 app reserves 16 MiB of virtual capacity only on first
  allocation; reservation allocates no backing frames. Small allocations
  reuse and coalesce within pages. Large and over-aligned allocations use
  reusable page runs with alignment up to 2 MiB. One VM commit call is limited
  to 64 pages. Freed backing stays committed for reuse and returns at exact
  region release or process teardown. Fixed per-process allocator metadata is
  28,688 bytes; the heap does not commit its full virtual limit.
- The real ring-3 VM proof reserves four pages, verifies both guards and an
  uncommitted page fail pointer validation, commits two zeroed pages, checks
  per-region/global accounting, refuses duplicate commit and a wrong-kind cap,
  reads data with RO and RX protections, rejects syscall output to protected
  pages, refuses W+X, restores RW with data preserved, releases the exact cap,
  rejects the stale cap and stale pointer, and observes committed/region totals
  return to the prior value.
- The same verified installed APB1 ELF is built with ScalableHeap as its Rust
  global allocator. Three real launches (explicit Open With, keyboard search,
  and pointer selection) reserve 16 MiB, commit 65 pages for a 256 KiB Vec,
  touch each page, drop/reallocate the same run, allocate at 64 KiB alignment,
  refuse an oversized request through try_reserve_exact, and close cleanly.
  Query reports 65 committed heap pages rather than the 4,096-page virtual
  capacity. The pre-heap to active-heap serial sample consumes 68 free frames
  across the VM/heap proof, including page-table overhead; process, regions,
  pages, maps, and caps return to identity baseline on teardown.
- The arena-runtime host suite passed 14/14 tests. Targeted Clippy passed with
  warnings denied for arena-runtime and the installed app. Release builds
  passed. The installed-registry guest passed on QEMU 10.0.11 / OVMF 2025.02
  with two simultaneous app instances and exact ProcessGroup teardown.
- test_m12_startup.py passed 7/7 M12 controls plus M1-M7 and M11. The current
  kernel test_m12_scale.py passed 32 live sessions, mutation-free 33rd
  refusal, 16-close/16-reuse, and full teardown in 169.6 seconds. It observed
  77 retained page-table frames; process, SharedRegion, page, map, and cap
  counts returned to baseline.
- Code review found that protection invalidation compared a process root to
  the boot kernel root, which could leave a stale writable TLB entry. The
  kernel now compares with the actual CR3 before invalidating the changed VA.
  The installed-app VM/heap guest passed after this correction.
- Rebuilt the exact EFI with SHA-256
  287794b6d397982fb1d080b2a1e8a722541d4ef8b31ca4d00c300098db963d82 and ran
  tools/stability_loop.sh 20: 20/20 fresh boots passed with zero failures in
  148 seconds. The receipt binds the boot set to that exact EFI. That VM/heap
  implementation was preserved in its checkpoint before multi-window work
  continued.

### Multiple independently backed windows per process T1

- ADR-0097 separates the compositor's authenticated window owner from its
  unique SharedRegion backing key. ABI-v2 `Create` still creates the primary
  surface; `CreateAdditional` asks the same badge-authenticated endpoint for
  a fresh bounded SharedRegion, broker-private snapshot, compositor handle,
  and independently owned publication state. Desktop returns the exact
  surface cap in the checked reply with rights limited to that object. The
  original startup capability inventory is unchanged.
- `DestroyWindow` retires one surface while keeping its process and sibling
  windows alive. Extra surface maps/snapshots are dropped on owner close or
  after the exact Process is reaped. Closing a primary window leaves the
  process-level shared backing and private snapshot available for a later
  window. The desktop remains bounded at 32 total ordinary windows; this work
  adds no kernel or startup resource limits.
- Each window has independent title, raster dimensions, damage generation,
  publication regions, transient surface, close state, focus/reveal animation,
  minimize/maximize policy, compositor slot, and snapshot. The client adapter
  keeps the original session backing for endpoint authentication and file I/O
  while using the returned exact cap only for an additional window surface.
- The compositor backing-key space is 64 entries (32 primary-session keys
  plus 32 additional-surface keys), while the Window Manager still admits at
  most 32 live windows total. Damage comparison now covers both key ranges.
  The initial guest race found that an extra window could be rendered before
  first publication and then remain blank because later damage ignored slots
  32–63. The host suite now checks high-slot first publication and partial
  damage against full-frame composition.
- `cargo test --manifest-path userspace/desktop/Cargo.toml --lib` passed
  84/84. `cargo check --manifest-path userspace/desktop/Cargo.toml --bin
  desktop --target x86_64-unknown-none` and the signed Phase-13 ELF release
  build passed.
- `tools/test_phase13_registry_guest.py` passed on QEMU 10.0.11 / OVMF 2025.02.
  The signed installed ELF created three ordinary windows in one process and
  published three different guest-visible surface markers; a second process
  did the same concurrently. The existing document Open With, read-only file
  cap, VM and heap assertions passed. The fixture closed windows one at a time:
  after each additional window closed, its process and sibling windows stayed
  live, with exactly two SharedRegions, 940 pages, and three maps reclaimed.
  Closing each last window then reaped its process; process, region, page, map,
  and cap identity counts returned to the boot baseline. The green guest took
  9.9 seconds after image build.
- An initial resource-close attempt caught a reply-cap cleanup issue: the
  temporary transferred surface cap needs DESTROY so Desktop can drop its own
  copy after the checked reply, and the receiver needs it to release that
  exact extra surface. The cap cannot select or operate on another window.
  After fixing rights, individual window close and full process teardown
  returned all identity counts to baseline.

### Multiple-window preservation T2

- `cargo test --manifest-path userspace/desktop/Cargo.toml --lib`: 84/84.
- `tools/test_m10_desktop.py`: dock launch, owned raster, keyboard/theme,
  drag, close, and relaunch passed.
- `tools/test_m12_scale.py`: 32 live ordinary sessions passed with a
  mutation-free 33rd refusal, 16-close/16-reuse, and full cleanup. It observed
  77 retained page-table frames; all process, SharedRegion, page, map, and cap
  identity counts returned to the Phase-12 baseline. The full guest run took
  180.1 seconds. At the refused 33rd launch, the exact inventory was 47
  processes, 66 regions, 30,550 pages, 112 maps, and 110 occupied cap slots;
  the same 77 page-table frames remained after complete teardown.
- The source was rebuilt into EFI SHA-256
  `1d77db78d33fe22b5c40f1758bd3c71b46c830a21f18a6e47c94b81b976c3c2a`.
  `tools/stability_loop.sh 10` passed 10/10 clean boots, zero failures, in 76
  seconds. Its receipt contains the same EFI SHA-256 and `10/10`.
- `git diff --check` passed. The multi-window T2 checkpoint was committed as
  `3fbdc50` and pushed to `origin/arena/phase13-native-app-maturity`; the
  working tree was clean before this receipt update. Development continues on
  the same branch.

### Signed headless installed application T1

- Added a no-window installed-app launch branch after the same receiver-side
  APB1 registry refresh and exact Image resolution used by windowed apps. The
  startup ABI carries the signed headless policy bit and a unique generation.
  Desktop inherits only the startup block and that instance's notification
  capability attenuated to `READ|WRITE`; it grants no Desktop endpoint,
  surface, filesystem root, or document File. No kernel or registry bounds
  changed. ADR-0098 records the boundary and the remaining manager lifecycle
  work.
- Added a signed APB1 guest fixture whose ring-3 runtime verifies the complete
  startup cap table, arms and waits on its real timer, then exits. Desktop
  observes and reaps the child through the existing exact Process-capability
  group. The fixture proves no compositor window was created and the child did
  not inherit a surface or Desktop endpoint.
- `python3 tools/test_phase13_registry_guest.py` passed on QEMU 10.0.11 / OVMF
  2025.02 in 14.3 seconds after the image build. It installed two distinct
  signed APB1 apps; preserved keyboard/search/pointer launch of the windowed
  app; proved two concurrent instances with three independently backed
  windows each; proved read-only document handoff; then installed and ran the
  headless app from All Applications. The headless app's 3-second timer
  completed, its child exited, and Desktop reaped the Process.
- Measured `(free frames, process records, processes, SharedRegions, region
  pages, maps, caps)` at the settled pre-launch baseline:
  `(114244, 15, 15, 2, 470, 4, 45)`. While headless was live:
  `(114203, 16, 16, 2, 470, 4, 46)`. The active process added no SharedRegion,
  region page, or mapping; only one manager Process cap was added. After timer
  exit and Process-cap reap the measured tuple returned exactly to
  `(114244, 15, 15, 2, 470, 4, 45)`, including free frames.
- The first attempt exposed that a 10-second test wait was too close to the
  guest timer's exact deadline. The fixture now uses a 3-second timer and
  separately waits for the real completion marker and resource teardown; the
  corrected guest passed cleanly.

### Headless lifecycle preservation T2

- Host suites passed: Desktop 84/84, arena-platform 34/34, and arena-runtime
  14/14. Release Clippy with warnings denied passed for the new headless guest;
  the Desktop binary check and targeted Clippy passed. Formatting and
  `git diff --check` passed.
- `tools/test_m10_desktop.py` passed dock launch, owned raster, keyboard/theme,
  drag, close, and relaunch. `tools/test_m12_scale.py` passed 32 sessions,
  mutation-free 33rd refusal, 16-close/16-reuse, and full cleanup in 188.8
  seconds. It observed 77 retained page-table frames; process, SharedRegion,
  region page, map, and cap identity counts returned to baseline. Its measured
  `(free frames, records, processes, SharedRegions, pages, maps, caps)` were
  `(114258,15,15,2,470,4,45)` at baseline,
  `(78311,47,47,66,30550,110,109)` at full use,
  `(96246,31,31,34,15510,58,77)` after closing half,
  `(78310,47,47,66,30550,112,109)` after reuse, and
  `(114181,15,15,2,470,4,45)` after teardown. The 77-frame decrease is the
  retained page-table allocation already identified by the Phase-12 scale
  proof.
- Rebuilt the exact desktop EFI at SHA-256
  `fca6c3f041c37503392c539dbe09a3940bdba4fcdc2a36d67698afe468a418a3`.
  `tools/stability_loop.sh 10` passed 10/10 fresh boots with zero failures in
  77 seconds. The receipt binds all ten green M1-M12 runs to that exact EFI.
- The headless launch adds no new kernel bound and leaves the Phase-12 low-32
  manager accounting, exact 128-slot cap table, and observable slot-127
  authority unchanged. This source checkpoint is ready to preserve; helper
  processes, full AppInstance lifecycle, streams, user
  threads, synchronization, launcher favorites, mixed-load pressure, and
  final qualification remain outstanding.

### Per-AppInstance ProcessGroup T2

- Replaced Desktop's single global child group with one bounded
  `ProcessGroup<ChildProcess, 4>` per occupied app-instance/session slot.
  Primary launch stores its generation-safe group handle in that owner's row;
  all liveness and teardown use the group's held Process capabilities. App
  retirement drains the complete group before releasing the row, so future
  helpers cannot be orphaned when the primary exits. Session indices and
  application IDs remain descriptive routing only.
- Desktop library tests passed 84/84. The desktop binary check passed, and
  targeted Clippy passed with the repository's existing
  `len_without_is_empty` and `if_same_then_else` allowances. The signed APB1
  guest passed: two instances with three windows each, read-only document
  handoff, one headless instance, and exact teardown. The 32-session scale
  guest passed with the 33rd launch refused without mutation and full cleanup.
- Scale receipt `(free frames, records, processes, SharedRegions, pages,
  maps, caps)`: baseline `(114256,15,15,2,470,4,45)`, full
  `(78309,47,47,66,30550,110,109)`, half-close
  `(96244,31,31,34,15510,58,77)`, reused-full
  `(78308,47,47,66,30550,111,109)`, final
  `(114179,15,15,2,470,4,45)`. The 77 retained page-table frames match the
  existing scale behavior; all identity-bearing counts returned to baseline.
- `tools/stability_loop.sh 10` passed 10/10 clean M1-M12 boots in 75 seconds,
  zero failures, for EFI SHA-256
  `c09a41221750958922861712236db5e459bd07739f170ff857cb86fcaa8ecd7e`.
  This checkpoint is preserved in commit `af9946f`; helpers, streams, user
  threads, synchronization, launcher favorites, mixed-load pressure, and
  final qualification remain open.

### Process-cap exit status T1

- Added ADR-0100 and syscall 61, `SYS_PROC_STATUS`. The kernel records the
  final thread's full `u64` status once in the Process record, makes it
  readable only through a live exact Process/READ cap, and drops the record at
  reap. Output pointers must be present and writable. A final user fault is
  reported as `0x100 + vector`; status zero is a normal successful exit.
- `arena-process::ChildProcess` and `ProcessGroup` expose member-scoped status
  and a wait helper that treats notification wakes only as hints, rechecking
  the held Process cap after each wake. Desktop logs the primary status before
  group cleanup. This is lifecycle data, not a readiness proof; ADR-0049's
  separately authenticated protocol-result requirement remains.
- The M8 guest passed live/running status, foreign read-only Process
  observation, wrong-kind and out-of-range refusals, invalid output pointer,
  exact child status 42, and stale-cap refusal after reap. The signed APB1
  registry guest passed on QEMU 10.0.11 / OVMF 2025.02 with document handoff,
  two instances owning three independent windows each, window-by-window
  teardown, headless timer exit status 42, and identity-resource counts back
  at baseline.
- The first updated registry oracle assumed the generic `status=42` line
  appeared only for the headless app. The guest correctly emitted the same
  status for the document launch and both windowed instances. The oracle now
  checks the per-headless-launch increment and exactly four total status
  receipts for the four fixture processes; the complete guest then passed.
- Kernel target check, arena-process host tests (8/8), Desktop and Shell target
  checks passed. The M8 and Phase-13 registry guests passed. Process-record
  and final-thread exit code changed, so the affected T3 regression set and
  20 clean boots are recorded below before preservation.

### Process-cap exit status preservation T3

- `python3 tools/test_m12_startup.py` passed 7/7 M12 checks plus M1–M7 and
  M11 in the same boot, including five startup refusals, Process-cap child
  wait/reap, exact TLS/heap behavior, and teardown accounting.
- `python3 tools/test_m12_scale.py` passed 32 live sessions, a mutation-free
  33rd refusal, 16-close/16-reuse, and exact identity-resource teardown. Its
  `(free frames, process records, processes, SharedRegions, pages, maps, caps)`
  were baseline `(114255,15,15,2,470,4,45)`, full
  `(78308,47,47,66,30550,111,109)`, half
  `(96243,31,31,34,15510,58,77)`, reused-full
  `(78307,47,47,66,30550,112,109)`, and final
  `(114178,15,15,2,470,4,45)`. Process, region, page, map, and cap counts
  returned to baseline. The 77-frame decrease is retained page-table memory,
  matching the existing scale behavior.
- The signed installed registry guest and `tools/test_m8_lifecycle.py` both
  passed. `arena-process` passed 8/8 host tests, Desktop passed 84/84 host
  tests, targeted Desktop Clippy passed, kernel target check passed, and the
  Shell target check passed with its existing dead-code warnings.
- `tools/build.sh --image` rebuilt the exact source as EFI SHA-256
  `b5de1d636f0542ca059a5687adf4740641e10fb32f843e80145de4b720c6e27a`.
  `tools/stability_loop.sh 20` passed 20/20 fresh full-suite boots with zero
  failures in 154 seconds; the receipt binds all 20 boots to that EFI hash.
- This is a green T3 preservation checkpoint for stable Process-cap exit
  status. Signed helper allowlisting, byte streams, persisted launcher
  favorites, user threads, synchronization, mixed-load pressure, and Phase-13
  final qualification remain open.
