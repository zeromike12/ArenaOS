# Phase 12 — application platform plan

**Parent qualified Phase-11 archive commit:** `a077ee4a929a0271a28308ac2c7bb28fd2597aae`.
**Qualified Phase-11 engineering source:** `fd7f481b46df6288708e14c5f898f6f00a3d6b02`.
**Session branch:** `arena/e3c48ce6-arenaos` (Arena binds this worktree to this branch; do not switch branches).
**Architecture record:** accepted ADR-0080. **Audit:** `ARCHITECTURE-AUDIT.md`.

This phase is deliberately not a visual redesign and not Linux compatibility.
The ordering is dependency-driven; later steps do not start by inflating
bounds or adding a kernel syscall table for an external ABI.

## Qualification gates

A checkpoint is not Phase-12 completion. Every new subsystem must have
host-side tests, a real guest proof against the exact built artifact, a
negative/mutation control for its authority boundary, historical regression
coverage, and a source/image/hash receipt. Capacity refusal must be explicit
and mutation-free. The Phase-11 qualified behavior is invariant.

## Work sequence

### 12.0 — Audit and architecture

- [x] Inspect accepted Phase-11 source, ADRs, code and measurements.
- [x] Record actual process, thread, cap, session, window, Image, ELF, package,
  startup, VM, TLS, stream, registry, and foreign-ABI state.
- [x] Accept the platform/security umbrella decision (ADR-0080).
- [x] Reconcile the one-page 800×600 shared-session discrepancy: the
  `tools/test_m11_wm.py` live receipt is 471/469, matching the current code;
  the earlier ADR-0075/Phase-11 470/469 value remains historical evidence.
- [ ] Derive the complete baseline boot inventory from resource counters and
  exact process/cap ownership, including dynamic Images and page tables.

### 12.1 — Application instance/process/window model v2

- [x] Introduce a no_std, host-testable metadata model for manifest-derived
  app instances, exact Process-cap-slot bookkeeping, and generation-safe
  windows; no kernel app/session/window object. Host-only model limits are 16
  instances, 4 process members/group, and 32 ordinary windows.
- [ ] Wire the model to exact held Process caps and the desktop broker; remove
  the “one backing ⇒ one window” limit using distinct per-window
  content/surface ownership or a formally bounded subsurface protocol.
- [ ] Support zero-window/background instances and helper-process membership.
- [ ] Measure and preflight a bounded ≥16 application/process instances and
  ≥32 ordinary windows on the 512 MiB QEMU profile. Include 800×600 and
  1024×768; prove refusal is mutation-free.
- [ ] Prove open/close/half-close/relaunch/helper churn returns frames, maps,
  caps, notifications, records, endpoints, watches, and timers to the
  expected baseline.

### 12.2 — Installed registry, launcher, preferences, associations

- [x] Add a host rebuild path that scans immutable AFS2 app/version namespaces,
  verifies current policy and every signed file, rejects extra files/directories,
  and atomically swaps the descriptive catalog only after a complete scan.
- [ ] Wire that scan to trusted boot-service authority; keep descriptive IDs
  separate from execution authority.
- [ ] All Applications launcher/search from the installed catalog (not
  hard-coded app-kind arrays), keyboard/pointer launch, running-state
  indication, explicit capacity refusal, favorites/running dock, and persisted
  AFS2 pin state.
- [ ] Content-type handler list, persistent default, Open With, missing-handler
  and uninstall cleanup. A chooser-held document capability must be separately
  offered to the selected instance; association strings grant no file access.
- [ ] Host model RED controls for name-as-authority, app/window ID reuse, stale
  running state, and association-without-document-cap.

### 12.3 — APB1 signed application bundles

- [x] Freeze canonical manifest/header/file-table encoding and exact bounds in
  format ADR-0081 before installer integration. APKG v1 remains accepted and
  unchanged.
- [x] Use the existing audited Ed25519/SHA-256 closure; add the no_std
  `arena-platform-core` verifier and initial independent host fixtures.
- [x] Enforce canonical relative paths, sorted/no duplicate or ancestor-
  conflicting file names, exact payload accounting, 64-file bound, per-file
  and total installed-byte bounds, and immutable version-path collision refusal.
- [x] Add a host-tested AFS2-engine install core: stream files into private
  staging, persist signed metadata/signature without duplicating payload,
  reconstruct and verify durable readback, then atomically rename to the
  installed version path. A fixed-depth janitor can purge abandoned stages.
- [x] Host RED controls cover signature/hash/path/canonical/bounds failures,
  wrong key, source mutation, reserved metadata-path collision, corrupt
  installed bytes/tree extras, duplicate version, every AFS2 write prefix,
  and remount/reverify or cleanup.
- [x] Add host policy binding compatible with the existing APKG v1
  `Chain`/`Policy` semantics: select by package-ID namespace and signer ID,
  enforce current subordinate/minimum/revocation policy, and bind the
  authenticated identity/full digest through staged readback.
- [ ] Load that chain from receiver-verified persistent policy authority and
  integrate installer/registry into protected AFS2/filesd guest service;
  prove `/System` denial without the exact install grant.
- [ ] Guest RED controls for interrupted install/activation, policy denial,
  corrupted installed records, and the exact shipped artifact.

### 12.4 — Native startup ABI v2

- [x] Freeze the additive bounded entry/record contract in ADR-0083: one
  explicit read-only startup SharedRegion in slot 0, five-cap spawn bound
  (four additional descriptors), exact 4 KiB stack/RSP, and fail-closed parse.
- [x] Add a no-alloc host codec for canonical argv/env/cap descriptors, CWD and
  optional stdio references, page/clock and executable/base facts; descriptive
  values never grant authority. An independent Python oracle and shared golden
  page provide cross-language roundtrip/mutation controls.
- [x] Add the reusable `arena-runtime` entry gate: copy into fixed private BSS,
  unmap/destroy slot 0, parse without heap allocation, and compare each exact
  live child cap before exposing slices or calling application code.
- [x] Independent ring-3 guest verifies actual startup/Notification caps,
  argv/env, entry/base, initial RSP/RFLAGS, and one-shot cleanup. Four RED
  controls (listed-cap rights, slot-0 rights, page count, malformed bytes)
  refuse before application entry and return frames/processes/maps/regions to
  baseline. The 512 MiB exact-artifact boot is `tools/test_m12_startup.py`.
- [x] No ambient path authority, hidden entry allocator, or undocumented
  inherited capability in the tested ABI-v2 path. Production app-manager
  and installed-registry launch integration remains open.

### 12.5 — Native runtime, heap, VM and TLS

- [x] Add reusable no_std `arena-runtime` above `arena-lib` (not POSIX); UI
  toolkit remains a higher layer.
- [x] Provide checked bounded allocation through the explicit `BoundedHeap`
  `GlobalAlloc`: max 32 mapped 4 KiB pages, requests through 4,080 bytes and
  16-byte alignment, coalescing/reuse, and null on unsupported layout, frame
  pressure, map pressure or bound. M12 fills 32 pages, refuses page 33 without
  mutation, preserves an explicitly held cap in reserved slot 63, then exits
  and restores frames/maps exactly. ADR-0084 records the cap-slot, page-scrub
  and teardown contract. This is a bounded heap, not VM.
- [ ] Design minimal native reserve/commit/release/protection/guard operations
  in a VM ADR only if current Untyped/SharedRegion APIs cannot implement them.
- [x] Add the native x86-64 TCB/FS.base contract in ADR-0085: `SYS_TLS_SET`
  validates 16-byte alignment, exact current-thread user mapping, writable+NX
  ordinary RAM, and preserves GS/`swapgs`. Scheduler saves/restores FS.base per
  kernel thread. The M12 ring-3 guest rejects read-only/kernel-half bases and
  survives a timer block/wakeup across a kernel-thread handoff.
- [ ] User-thread creation/TLS uniqueness and shared-heap syscall-pointer
  validation remain open; do not enable threads on this allocator until
  user-region ownership is made process-wide.

### 12.6 — Threads and synchronization

- [ ] Capability-safe bounded user-thread create, explicit stack and TLS,
  exit/join/wait, process teardown, and no frame/stack leaks.
- [ ] Native mutex, condition/wait, once and thread-local state. Any generic
  wait/wake memory primitive gets a separate ADR and is not called Linux futex.
- [ ] Real multi-thread guest tests: isolation, TLS uniqueness, synchronization,
  teardown, table-full mutation-free refusal, owner/process death.

### 12.7 — Child/helper process groups

- [ ] Mediated allowlisted helper spawn with argv/env and explicit attenuated
  cap list; no ambient inheritance.
- [ ] Retain exact Process caps at the app manager; process group membership
  is not a PID/app-name lookup. Wait/status/terminate authorization are tested.
- [ ] Prove parent death, child death during wait, helper restart, cap teardown,
  stale Process identity and unauthorized helper requests.

### 12.8 — Streams/pipes and startup stdio

- [ ] Specify finite capacity, endpoint authority, read/write, partial transfer,
  EOF, close, process-death cleanup, block/wakeup notification, and writer
  multiplicity. Prefer shared ring + explicit notifications for bulk data.
- [ ] Prove full-buffer backpressure, faster producer, waiting consumer,
  producer death => EOF, consumer death, partial I/O and no false success.
- [ ] Use streams for native stdin/stdout/stderr after the runtime contract is
  frozen; console remains one client/service, not a universal stream syscall.

### 12.9 — Userspace integer runtime handle table

- [ ] Optional userspace-only integer handle map over Stream, file/directory
  capabilities, event/timer, socket client, and child-process objects.
- [ ] Test duplication, close, invalid/full table, stale generation reuse,
  rights attenuation and teardown. Kernel capability slots remain native
  authority; an integer by itself has no cross-process meaning.

### 12.10 — PIE and executable modernization

- [ ] Add native `ET_DYN` PIE only if loader audit permits, with fixed accepted
  relocation subset (initially relative relocations), variable/randomized base,
  checked non-overlap, W^X, NX, stack guard gap, and explicit failure policy.
- [ ] Use held rngd authority for entropy; prove varying bases, prohibited
  regions, no overlap, and missing entropy behavior. Existing ET_EXEC stays
  supported and regression-tested. No dynamic linker/PT_INTERP.

### 12.11 — Foreign ABI syscall proxy and ProcessMemory

- [ ] Freeze a generic foreign-personality/proxy ADR. Personality chosen only
  by trusted launch authority; immutable for process lifetime; every foreign
  syscall is proxied and can never invoke the native dispatch table.
- [ ] Bounded request includes syscall number, six raw arguments, receiver-
  verified process identity/authority, and cancellation/death state; bounded
  reply result plus only a designed action enum.
- [ ] Hold exact userspace compat-server authority. Server death fails clients
  closed; no kernel Linux syscall numbers/semantics.
- [ ] Add exact child-scoped ProcessMemory capability for checked `copy_from`
  /`copy_to` with READ/WRITE rights, user-range validation, no arbitrary
  physical access, no PID lookup, and process-death revocation.
- [ ] Tiny deliberately foreign test ABI through `compatd`: write, time/clock,
  exit; negative control as native; death/cancel/invalid-pointer/oversized
  length/forged identity/wrong process/revocation and personality-mutation
  tests. No Linux compatibility claim.

### 12.12/12.13 — Desktop discovery and interaction

- [ ] Connect Files/Desktop open-by-type to registered app handlers.
- [ ] Keep visual identity; add launcher/search and app-window list/show-all
  using existing interaction primitives, with keyboard and pointer operation.
- [ ] Demonstrate real registered application launch and document capability
  handoff from the extracted checkpoint.

### 12.14 — Scalability fixture and resource ledger

- [ ] Pressure fixture: ≥16 app/process instances, ≥32 windows, multi-window
  instances, headless helpers, streams/pipes, file caps, timers/notifications.
- [ ] Record process/thread/window/SharedRegion/page/map/cap/endpoint/notif/
  timer/watch/frame/dynamic-Image high-water values.
- [ ] Open many, close half, relaunch, spawn/exit helpers, close all; compare
  resource baselines and high-water with explicit table budgets.

## Final qualification and deliverable

Only after all gates above:

1. All historical suites and all new host/guest/mutation suites pass; formatting
   and static checks pass.
2. Build exact shipping EFI and run artifact-bound `tools/stability_loop.sh 100`
   from zero, with registry-ready, native launch, multi-instance, startup,
   document or association, stream/child, and clean shutdown evidence.
3. Build `phase12-complete` with exact EFI, firmware, AFS2 image/template,
   APB1 fixtures, boot scripts, hashes, docs, suite/stability receipts.
4. Independently extract and boot only the archive contents; exercise All Apps,
   real registered app, multi-window/multi-process app, document association,
   stream/child, clean shutdown and (if practical) foreign fixture.
5. Record archive and EFI SHA-256 plus the exact final source receipt in
   `FINAL-REPORT.md` and `COMPATIBILITY-HANDOFF.md`.
