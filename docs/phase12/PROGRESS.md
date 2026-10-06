# Phase 12 progress and receipts

- **Current source base:** qualified Phase-11 archive tip (`a077ee4a929a0271a28308ac2c7bb28fd2597aae`)
- **Qualified engineering source:** `fd7f481b46df6288708e14c5f898f6f00a3d6b02`
- **Working branch:** `arena/e3c48ce6-arenaos` (Arena session-bound; no branch switch)
**Phase-12 status:** source audit and umbrella ADR complete. APB1 format and
AFS2 staged-install decisions are recorded; the no_std lifecycle/catalog and
APB1 verifier/AFS2 install cores, exact-tree registry reconstruction, and the
ADR-0083 startup codec pass their current host gates. The host installer now
also streams its APB1 input from a regular file in the same AFS2 `Volume` that
owns the protected destination, without whole-archive buffering or aliased
volume borrows. A host signer resolver binds to APKG v1 `Chain` semantics, but
neither resolver, installer, nor registry scan is integrated into the desktop
or protected guest filesd service.
The reusable no_std startup runtime has host/target validation and an
independent M12 ring-3 proof covering exact startup-capability inventory
(including refusal of an unlisted live cap), the generation-safe handle-table
core, real attenuated cap copy/close, Process-cap child spawn/wait/reap,
basic per-thread FS-base TLS, bounded heap behavior, and the live 32-caller IPC
queue boundary. Its capability-native `arena-process` lifecycle crate is used
by the protected Desktop manager for real child ownership, liveness, and finish
through a generation-safe group. The official M12 scale guest now passes a
32-live-session built-in-app capacity checkpoint: all six kinds, exact refusal
inventory, close/reuse, window operations, and teardown. This is not yet the
required multi-window-per-app/headless-helper qualification, nor signed/dynamic
app or broad lifecycle qualification. Persistent receiver-policy authority,
protected install/registry/launcher, multi-window and headless process-group
paths, streams, user-thread lifecycle, and foreign-proxy paths remain open.
This remains a live ledger, not a completion claim.

## 12.0 — Source audit / ADR-0080

- Audited actual accepted ADRs, Phase-11 sources and historical tests before
  making new design decisions.
- `ARCHITECTURE-AUDIT.md` records actual process/session/window/app/package/
  ELF/startup/VM/TLS/thread/stream/registry/proxy boundaries and resource
  consequences.
- Accepted ADR-0080: app definitions, instances, process groups and windows
  are separate userspace policy concepts; ArenaOS remains capability-native;
  APKG v1 remains unchanged; any foreign ABI is a distinct trusted-launch-
  selected userspace proxy personality.
- Accepted ADR-0081 freezes APB1 metadata/bundle bytes; it does not yet accept
  signer-policy, AFS2 transaction or launcher integration.
- No kernel or capacity limit has been raised.

### Source-derived bounds

| Resource | Current bound / observation | Source |
|---|---:|---|
| Processes | 32 | `kernel/kernel/src/proc.rs` |
| Kernel/user threads | 64; 8 frames/32 KiB per kernel stack | `kernel/kernel/src/sched/mod.rs` |
| Spawn records | 32; exited children remain until held-Process finish | `kernel/kernel/src/spawn.rs` |
| Per-process capabilities | 64 | `kernel/kernel/src/cap.rs` |
| Dynamic Image storage | 2 slots × 4,096 bytes; dynamic image pages max 16 + stack | `kernel/kernel/src/image_registry.rs`, ADR-0055 |
| Unretired dynamic children | 4 system-wide | `kernel/kernel/src/spawn.rs`, ADR-0064 |
| Desktop ordinary windows/sessions | 12; one backing/process per session | `userspace/desktop/src/model.rs`, broker |
| Shared regions/pages/maps | 32 slots, 1,024 pages/region, 20,480 aggregate pages, 64 maps | `kernel/kernel/src/shared.rs` |
| IPC endpoints / notifications | 16 / 31; endpoint queue depth 16 | `kernel/kernel/src/ipc.rs` |
| ELF | static ET_EXEC only, ≤8 headers/segments; no PT_INTERP/PIE | `kernel/kernel/src/elf.rs`, ADR-0016 |
| APKG v1 | 128-byte manifest + 1–4,096-byte payload + 64-byte signature; max file 4,288 bytes | `userspace/package.rs`, ADR-0053 |
| Desktop session caps | settled high-water 60/61 at twelve sessions; 64 slots | Phase-11 `PROGRESS.md`, ADR-0075 |

The table above records the audited Phase-11 source, before the narrowly
measured Phase-12 capacity deltas. Current M12 limits and evidence are tracked
below; unrelated limits remain unchanged.

## 12.0.1 — Live Phase-11 resource baseline

`tools/test_m11_wm.py` passed on 2026-10-06 UTC against the exact artifacts
whose SHA-256 values and serial evidence are retained in
[`BASELINE-RECEIPT.md`](BASELINE-RECEIPT.md). The desktop's actual 800×600
receipt is **471 shared / 469 snapshot pages**. This reconciles the current
source formula; the accepted Phase-11 ADR/progress 470/469 figure remains
historical and is not rewritten.

Settled baseline was `115127/15/15/1/469/2/23`, in order:
free frames / spawn records / live processes / SharedRegion slots / shared
pages / mappings / broker cap occupancy. Twelve sessions peaked at
`101891/27/27/25/11749/38/47`; closing them restored every non-frame counter
to its baseline. Twenty-four free frames remain, consistent with two retained
broker page-table frames per warmed window VA slot and existing Phase-11
three-/two-window proofs. The 12-session count consumes 11,280 shared pages;
the 13,236-frame free-count drop includes other per-process/page-table/image
costs whose complete inventory is still open.

The thirteenth launch returned status -4 with records, processes, regions,
pages, maps and caps unchanged. The forced sample showed eight *more* free
frames than the preceding peak sample; the Phase-11 test does not compare the
frame field across refusal. This timing/page-table observation is recorded as
an unresolved part of the all-resource refusal budget, not misreported as a
Phase-12 proof. No limits were mutated.

## 12.1 — Host lifecycle model (integration open)

- Added `userspace/arena-platform/src/lifecycle.rs`: no_std, fixed-capacity
  app-instance, process-group, and window metadata. The table bounds 32 app
  instances, four process members/group, and 32 ordinary windows. Its instance
  slot count is shared with ABI-v2; windows remain a separate identity/table.
  Handles are generation checked; primary/helper membership records exact
  manager cap-slot indices; owner checks and teardown are explicit. This model
  does not spawn, retain capabilities, allocate surfaces, or authorize broker
  requests.
- Five lifecycle host tests cover stale/reused references, helper membership,
  owner-only window close, teardown, the 32/32 target, and mutation-free
  refusal. Desktop and kernel were unchanged at that checkpoint; guest proof
  for lifecycle-table wiring remains open.

## 12.3 — APB1 format and AFS2 host install foundation

- Accepted ADR-0081 freezes the canonical 64-byte header, 512-byte AMF1
  manifest, up to 64 sorted 144-byte file rows, 64-byte Ed25519 signature,
  exact envelope, 16 MiB/file, 32 MiB total, and 4 KiB streaming bound. The
  verifier workspace is 13,918 bytes; it must be persistent/static or
  runtime-managed, never an initial-stack local.
- Accepted ADR-0082 chooses a protected private staging directory and one
  AFS2 atomic directory rename as the immutable install visibility boundary.
  `APB1.record` persists signed metadata/signature without duplicating the
  packed payload. An installed-source adapter reconstructs that stream from
  the record and extracted AFS2 files so the verifier can recheck readback.
- The no-alloc `arena-platform-core` now includes the verifier, descriptive
  registry/association model, host lifecycle model, APKG `Chain`-compatible
  APB1 key resolver, AFS2 install/verify/janitor core, and atomic registry
  rebuild from the immutable install namespace. The rebuild rechecks policy,
  signature and every listed-file hash, rejects extra/unlisted files and dirs,
  binds `<app-id>/<canonical-decimal-version>`, and swaps the visible catalog
  only after a complete scan. `install_with_policy` binds the unauthenticated
  claim to the verified package-ID/signer/version/full digest, enforces current
  minimum/revocation policy, and repeats the binding on durable readback. The
  root key remains the explicitly public Phase-8 test key; production key
  provisioning is not claimed. Installer scratch is 13,952 bytes, cleanup
  scratch 10,368, and registry tree scratch about 9,984; with the 13,918-byte
  verifier the bounded install/scan workspaces total about 48,222 bytes.
  Double-buffering the 64-entry catalog also requires two manager-owned
  registry tables. Keep these in managed/static memory, never the initial
  stack. No caps are held here.
- Rust host tests: 34/34 pass. AFS2 tests stream a signed multi-file tree,
  reverify installed payloads, rebuild an exact-tree manifest-derived registry,
  reject unsigned extra files, reserved metadata-path collisions, wrong keys,
  policy downgrade/revoke/namespace denial before filesystem writes, refuse a
  duplicate version without changing the disk image, detect source mutation
  and installed-byte corruption, exercise every block/commit write prefix
  through remount, and purge orphan staging trees. Path-prefix conflict and
  reserved-record path are signed hostile controls in Rust and the independent
  Python oracle.
- Independent Python/OpenSSL APB1 oracle: 6/6 pass. APKG v1 independent
  receiver/wire tests: 6/6 pass; APKG bytes/source remain unchanged.
- Crate formatting, Clippy with warnings denied, and `x86_64-unknown-none`
  release build pass. These are host/target foundations only: persistent
  receiver-policy loading, filesd/service integration, guest boot registry
  scan, real guest install proof, and guest `/System` denial remain open.

## 12.4 — Native startup ABI v2, runtime entry gate, and guest proof

- Accepted ADR-0083 defines a userspace-owned startup-page ABI without changing
  legacy spawn: one read-only `SharedRegion` in child slot 0, existing five-cap
  spawn maximum (four additional explicit descriptors), exact one-page/RSP
  contract, and no ambient argv/env/CWD grants. Its descriptive instance-slot
  range is 0..31, matching the lifecycle table and 32-slot Desktop session
  envelope; it is not folded modulo 16.
- Added `startup.rs` encoder/parser for the canonical 128-byte ARST header,
  bounded arg/env tables, exact cap kind/rights/role references, CWD and
  optional stdio descriptors, entry/load-base/page/clock facts, zero reserved
  and tail bytes, and a required one-page read-only transport cap. Five Rust
  tests and six independent Python tests cover roundtrip, malformed
  offsets/padding/identity, descriptor mismatch, capacity refusal,
  startup-cap geometry, runtime guest vectors, and mirrored occupancy-syscall/
  cap-bound constants. The shared 4 KiB pages are reproducible and byte-
  identical across codecs. Names and descriptor slots remain descriptive.
- Added no_std `userspace/arena-runtime`: it validates slot 0 as the exact
  one-page `SharedRegion/READ|DESTROY`, maps read-only, copies to private fixed
  BSS, unmaps/destroys the transport before application entry, parses without
  heap/initial-stack allocation, and compares each actual child cap kind and
  rights while ignoring descriptive object IDs. ADR-0086 adds metadata-free
  `SYS_CAP_OCCUPIED` and scans all 128 slots, requiring every listed capability
  to be present and every unlisted slot to be empty. The runtime host suite
  currently passes 13/13 tests; Clippy is clean and the independently linked
  proof image builds for `x86_64-unknown-none`. The separate process/handle
  crate adds the seven host tests recorded in 12.7.
- `tools/test_m12_startup.py` boots the exact artifact. The guest checks
  argv/env, actual Notification slot 1, entry/base, initial RSP, IF/DF, and
  successful startup-cap reclamation. Five RED cases—listed-cap rights, slot-0
  rights, wrong page count, bad magic, and an extra live MemoryPool cap—refuse
  before the application closure. Per-case frame/process/notification/map/
  SharedRegion snapshots return exactly to baseline: m12 PASS 7/7 (five
  startup RED controls plus the reserved heap-slot collision); m1–m7 and m11
  regression markers pass in the same boot. The proof image adds one explicitly
  bounded boot fixture only; process/cap/resource limits are unchanged.
- This remains an independent qualification guest rather than an installed-app
  manager proof. The built-in Desktop v2 launcher/runtime and badge dispatch
  have since been integrated, but require the targeted M12 Desktop guest and
  RED regressions recorded below. Installed registry/launcher discovery,
  dynamic native launch, general VM, user-thread lifecycle, and the broad
  runtime remain open; the basic FS-base TLS and bounded-heap proofs are
  recorded in 12.5 below.

## 12.5 — Bounded native heap and basic FS-base TLS

- Added ADR-0084 and `arena-runtime::heap::BoundedHeap`, a process-local Rust
  `GlobalAlloc` above the existing `SYS_ALLOC_FRAME`/`SYS_MAP_MEMORY` path.
  Allocation is lazy and limited to 32 pages × 4 KiB, request sizes ≤4,080 B,
  alignment ≤16 B; in-page headers split, coalesce and reuse blocks. Newly
  mapped pages are zeroed before first exposure. Unsupported layouts, frame or
  map pressure, and allocator capacity return null; failed mapping destroys
  the still-owned cap. Slot 63 is the explicit transient frame-cap slot.
- M12 positive guest actually fills all 32 pages, checks two endpoint bytes in
  each distinct allocation, verifies page 33 returns null, deallocates and
  reuses coalesced blocks, then exits. A separate live-cap RED control occupies
  slot 63 with an attenuated Notification; heap OOM preserves that exact cap.
  Per-case cap-slot and kernel frame/process/map/SharedRegion counters return
  to baseline after teardown. No ordinary
  frame unmap, general reserve/commit/protect API, or kernel limit change was
  added; mapped heap pages remain bounded until process death.
- The basic x86-64 TLS base is implemented as ADR-0085: `SYS_TLS_SET` accepts
  only aligned writable+NX ordinary pages in the caller's registered user map;
  scheduler state saves/restores FS.base per thread while GS stays dedicated to
  `swapgs`. The independent guest proves read-only/kernel-half refusal and
  TCB/sentinel survival across a timed block/wakeup; kernel checks the one-shot
  timer retires. Host tests cover the frozen TCB layout, heap capacity/OOM,
  zero-on-new-page, alignment, split/coalesce/reuse and invalid layouts.
  Clippy and the `x86_64-unknown-none` guest build pass.
  TLS is not yet tested across two user threads, and heap mappings remain
  per-thread in syscall pointer validation. Process-wide user-region ownership,
  VM semantics and actual user-thread creation remain open.

## 12.7 — Native child-process group foundation

- Added `arena-process::ChildProcess` over the existing `SYS_SPAWN`,
  `SYS_PROC_LIVE`, and `SYS_PROC_FINISH` ABI; `arena-runtime` re-exports the
  process and handle APIs. Spawn grants are an explicit, bounded list of at
  most five source-slot/rights pairs. The PID result is only diagnostic/
  correlation metadata: the wrapper scans actual held slots and retains the
  unique exact Process/READ|DESTROY capability; all liveness and finish calls
  use that cap slot. Exit badges are wake hints, never proof of exit or
  lifecycle authority.
- Added fixed-capacity `ProcessGroup<T, N>` over generation-safe runtime
  handles. Capacity is refused before calling the spawn closure. Explicit
  teardown stops live children and reaps exited children; failed cleanup leaves
  the exact member handle retryable. The group is single-owner, has no
  destructor syscalls, and must be explicitly shut down. The protected Desktop
  manager now uses this group for its real spawned applications; sessions hold
  only local generation handles, and liveness/retirement route through each
  exact Process cap. The subsequent 32-session built-in checkpoint is recorded
  in 12.15; the ProcessGroup-to-32-slot lifecycle-table wiring, multi-window
  ownership, headless-helper scale, parent-death/helper-restart policy, and
  multi-thread synchronization remain open.
- The separate `arena-process` suite (7) and `arena-runtime` suite (13) pass
  20/20 combined; runtime Clippy and the release proof build pass. Host tests
  cover full-group mutation-free refusal, stale handles, live-stop versus
  exited-reap selection, and failure retention. M12 spawns the proof
  BootImage as a child with no inherited caps, waits on an explicit exit
  notification, checks liveness via the actual Process cap, reaps it, and
  proves the group handle and cap slot are gone. The badge is only a wake hint;
  numeric PID values used as Process-cap slots are rejected. The same guest
  also proves one-member group capacity refusal before a second SYS_SPAWN.
- ADR-0087 records the lifecycle contract. M12 passes 7/7 and M1–M7/M11
  regressions pass in the same boot with exact resource return. The integrated
  Desktop path also passes `test_m10_apps.py`, `test_m10_dynamic.py`,
  `test_m11_wm.py`, and `test_m11_files.py`, including twelve real sessions,
  mixed builtin/dynamic children, thirteenth-session mutation-free refusal,
  normal/forced retirement, and filesystem authority regressions.

## 12.9 — Userspace runtime handle-table foundation

- Added the fixed-capacity no_std `arena-runtime::handles::HandleTable`. A
  32-bit value encodes only a process-local slot and generation; it is not a
  cap slot, PID, or transferable authority. Closing returns the held object so
  its owner can explicitly perform cap cleanup. Slot generations advance on
  reuse and retire instead of wrapping.
- `duplicate_with` reserves a free table slot before invoking the caller's
  object-copy/rights-attenuation closure. Full tables refuse before a kernel
  cap copy can occur; failed attenuation leaves table contents unchanged.
  The table does not mint or copy authority itself and requires exclusive
  mutable access; user-thread synchronization remains open.
- Four handle-table host tests cover stale alias refusal after reuse,
  invalid/full slots, generation exhaustion, attenuation, and
  copy-failure/capacity atomicity. A failed full-table insert returns the exact
  rejected owner value; the M12 guest proves this and that full-table
  duplication never calls its capability-copy path. Two capability-wrapper
  host tests cover requested-right validation;
  `arena-runtime::capabilities::HeldCapability` wraps actual
  occupied/described slots, attenuation-only `SYS_CAP_COPY`, and explicit
  destroy with owner recovery on failure. M12 copies a real Notification into
  slot 3, verifies source rights unchanged, and destroys the copy. Typed
  wrappers for streams/files/directories/events/timers/sockets remain open;
  Process-cap wrappers are implemented and integrated with the Desktop manager
  at the existing session limit. Parent-death policy and cross-thread
  synchronization remain open.

## 12.15 — IPC burst and 32-session built-in capacity checkpoint

On 2026-10-06, the official targeted commands completed successfully:

```sh
source tools/dev-env/env.sh
cargo fmt --manifest-path kernel/kernel/Cargo.toml
cargo fmt --manifest-path userspace/desktop/Cargo.toml
python3 -m py_compile tools/test_m12_scale.py tools/test_m12_startup.py tools/probe_capspace_layout.py
python3 tools/probe_capspace_layout.py
python3 tools/test_m12_scale.py
```

`probe_capspace_layout.py` confirms the on-target Phase-12 layouts:
`CapSpace<128>=3200 B`, `Process<128>=3248 B`, `Endpoint<32>=7728 B`,
16 endpoints = 123,648 B, and 64 notifications = 2,048 B. The scale test
rebuilt the exact guest and passed in 121.2 seconds with the M12 live 32-caller
queue proof. Queue depth 32 accepted and served callers 1–32 exactly once;
caller 33 received mutation-free `STATUS_BUSY`, and proof callers/threads/
endpoint returned to baseline.

The direct 512 MiB guest run then managed 32 live ordinary sessions across all
six built-in kinds. Its resource receipts, in the order free frames / spawn
records / processes / regions / pages / maps / Desktop cap occupancy, were:

- baseline: `114761/15/15/2/470/4/44`
- full: `78941/47/47/66/30550/112/108`
- half-closed: `96813/31/31/34/15510/58/76`
- full after 16 slot reuses: `78940/47/47/66/30550/113/108`
- final: `114684/15/15/2/470/4/44`

The final non-frame counters returned to baseline. The 77-frame delta is the
explicitly measured retained empty page-table high-water, not attributed to
an app leak. On the 33rd launch, both all seven resource receipt fields and
all 128 manager cap descriptors compared exactly equal before/after refusal;
the request's temporary lent capability had landed at both samples. The
workflow also audited 32 distinct client clocks, exact Process-cap and region
ownership, maximize/minimize, dock restore, normal-size restore, and final
session retirement. No app was evicted and no resource limit was relaxed.

This closes the **built-in one-window-per-process capacity checkpoint only**.
It does not establish multi-window-per-process ownership, 32 windows across
fewer than 32 app processes, headless/helper scale, signed/dynamic apps,
installed-app discovery, file-handler authority, streams, user threads,
parent-death cleanup, 1024×768 capacity, or broad lifecycle/stability gates.
Do not report Phase 12 complete.

## 12.16 — Built-in ABI-v2 launcher and badge dispatch (qualification pending)

- Built-in kinds now launch through `arena-runtime`'s startup gate and use
  per-session badged service endpoints. Child slot 0 is the read-only startup
  page, slot 1 the service BadgedEndpoint, slot 2 the writable surface, slot 3
  the private Notification, and optional slot 4 the explicit Monitor/filesd
  tail grant. Dynamic kind 255 remains on the legacy path. The startup codec
  lives in dependency-light `arena-startup-abi`; platform re-exports it, while
  Desktop/application/runtime avoid package-signature crypto dependencies.
- The previous `instance_slot = session_index % 16` mapping was invalid: ADR-
  0083 and the lifecycle model used 16 slots while ADR-0088's broker managed
  32 simultaneous built-in process/window sessions. The accepted ABI-v2
  descriptive slot range and lifecycle table are now 32, matching the
  32-session bound. Desktop writes the exact free row `i`, and a nonzero
  monotonic generation changes on reuse; no live slot aliases another. The
  window table remains separate, and the slot/generation never authenticate
  IPC. This is a bounded userspace contract update, not a kernel limit raise.
- Desktop receives `SYS_IPC_RECV_BADGED` and selects exactly one matching live
  session via a pure helper. Zero remains the legacy path; unknown, dead-owner,
  or duplicate badges reject without fallback. Bootstrap and trusted function
  rights come from the selected broker record; surface and filesd Offer caps
  are checked against its object/profile. The app-side audit tries endpoint
  re-minting, sends the previous session generation in caller-controlled words
  (the reply must still identify the current app), and sends the wrong object
  with its valid endpoint cap. The per-app guest marker is required by the
  updated scale test.
- The 32-slot lifecycle/startup host model and ABI oracle now test slot 31 as
  valid, slot 32 as mutation-free refusal, plus zero/unknown/dead/duplicate
  badge selection. These are not yet a guest qualification: run the exact
  M12 scale guest and affected M10/M11 regressions before calling the
  integration qualified. A previously built x86_64-unknown-none check/link
  predates these final slot/audit edits and must be rerun.
- Compatibility caveat: a separately shipped older startup-v2 consumer that
  hardcodes the former 0..15 ceiling will reject slots 16..31 before app
  entry. Current built-ins are rebuilt together. The future installed-app
  launcher must check each runtime's supported startup-slot range (or require
  an updated runtime) before selecting a high slot; dynamic ABI-v1 images are
  unaffected.

## 12.17 — Same-volume APB1 source seam and guest-boundary audit

- `arena-platform-core` now exposes `install_with_policy_from_volume_file()`.
  It validates the exact internal source object is a regular AFS2 file and
  streams bounded reads through the same `Volume` used for staging and the
  atomic install rename. The source is read in sequence with writes; no unsafe
  alias, second stale mount, or whole-bundle allocation is introduced. Existing
  `BundleSource` callers and APKG v1 decoding remain unchanged.
- Added host controls for successful same-volume multi-file installation,
  source-file immutability, and mutation-free refusal of a directory as a
  candidate. `arena-platform-core` host tests pass 31/31, and the no_std
  `x86_64-unknown-none` check passes.
- `SERVICE-INTEGRATION-AUDIT.md` records the current `packaged`, `servicemgr`,
  and `filesd` seams. `packaged` has exactly five startup grants and consumes
  every landed cap as its current Notification diagnostic marker; it has no
  AFS2 Volume. `filesd` owns the AFS2 Volume and has no APB1 install dispatch or
  receiver-verified persistent policy loader. `/System` currently has no
  ordinary caller record; the normal `/Users/user` badge remains the only
  user-facing AFS2 root.
- No guest IPC operation, install badge, extra startup/spawn grant, protected
  namespace mutation, or policy handoff was added. Architecture remains open:
  source cap ingress must be explicit, policy must be receiver-verified from
  persistent APKG v1 state, and `/System` denial must remain RED-tested before
  the guest service can be safely connected. No new resource high-water was
  measured.

## Qualification attempts and remaining gates

| Gate | Result |
|---|---|
| Phase-11 baseline guest `tools/test_m11_wm.py` | PASS; baseline only, not a Phase-12 application proof |
| APB1 independent Python/OpenSSL oracle | PASS 6/6 |
| APB1 + policy + lifecycle + AFS2 install/registry/startup Rust host tests | PASS 34/34 |
| APB1 clippy / format / no_std target build | PASS |
| Native startup ABI v2 Rust codec / independent Python oracle and ABI parity | PASS 5/5 + 6/6 |
| arena-process + arena-runtime startup/exact-cap-inventory/heap/TLS/handle/capability/process-group host tests / no_std builds | PASS 20/20 combined (7 + 13); runtime Clippy clean; guest image builds |
| Independent M12 ring-3 startup + exact cap inventory + FS-base TLS + real attenuated cap copy/close + Process-cap group spawn/wait/reap + 32-page heap, five startup REDs and heap-slot collision | PASS 7/7; exact frame/map/cap/timer/resource return; M1–M7 + M11 regression PASS |
| APKG v1 independent host tests | PASS 6/6; no wire reinterpretation |
| Desktop ProcessGroup integration: M10 apps/dynamic and M11 window/files regressions | PASS; twelve-session Phase-11 regressions and cleanup preserved |
| M12 IPC queue32 plus built-in 32-session resource/window guest `tools/test_m12_scale.py` | PASS; exact 32/33 queue boundary, 32 live apps, mutation-free cap/resource refusal, 16-close/16-reuse, window operations, full teardown; broader multi-window/helper/dynamic-app qualification remains open |
| Phase-12 protected filesd install/registry and lifecycle guest proof | NOT RUN / NOT INTEGRATED |
| Heap/VM/TLS, user threads, helper groups, streams/handles, PIE, foreign ABI, full resource inventory and Phase-12 full suite | NOT RUN |
| Artifact-bound 100/100 loop and extracted `phase12-complete` boot | NOT RUN |

The installed toolchain omits `rustdoc`; use `cargo test --lib` for this crate
(no doctests exist) rather than a default doctest invocation. This is a host
runner constraint, not a product behavior claim.

## Immediate next steps

1. Finish the complete 512 MiB inventory: process and record ownership, exact
   cap use, dynamic Image residency, page-table frames, physical frame headroom,
   maps/notifications/endpoints/timers/watches, and settled refusal samples.
   Do not raise limits before this budget closes.
2. Load the host-resolved APB1 chain from receiver-verified persistent policy
   authority; integrate install/readback/rename with protected AFS2/filesd
   authority, boot registry reconstruction, cleanup, and guest failure controls.
3. Integrate lifecycle and the installed registry with the app manager and
   compositor. Separate app/process-instance ownership from per-window
   surfaces; design independent publication and headless/helper accounting
   within ADR-0088's bounded 80-region, 36,864-page, 128-map and 128-cap
   envelope, then measure notification, page-table and frame headroom. Qualify
   800×600 and 1024×768 on the exact guest.
4. Continue through the remaining broad native runtime/heap/VM/TLS, threading,
   explicit helper groups, streams, userspace handles, PIE feasibility, and
   separately selected foreign proxy/ProcessMemory gates in `PLAN.md`.
5. Only after every gate, run the historical suite, artifact-bound 100/100
   stability loop, and extracted archive boot; publish hashes, logs and final
   receipts. Phase 12 is not complete.
