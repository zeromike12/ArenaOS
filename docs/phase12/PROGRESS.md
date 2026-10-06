# Phase 12 progress and receipts

- **Current source base:** qualified Phase-11 archive tip (`a077ee4a929a0271a28308ac2c7bb28fd2597aae`)
- **Qualified engineering source:** `fd7f481b46df6288708e14c5f898f6f00a3d6b02`
- **Working branch:** `arena/e3c48ce6-arenaos` (Arena session-bound; no branch switch)
**Phase-12 status:** source audit and umbrella ADR complete. APB1 format and
AFS2 staged-install decisions are recorded; the no_std lifecycle/catalog and
APB1 verifier/AFS2 install cores, exact-tree registry reconstruction, and the
ADR-0083 startup codec pass their current host gates. A host signer resolver
now binds to APKG v1 `Chain` semantics, but neither resolver, installer, nor
registry scan is integrated into the desktop or protected guest filesd service.
The reusable no_std startup runtime has host/target validation and an
independent M12 ring-3 proof covering startup validation, basic per-thread
FS-base TLS, and bounded heap behavior. It is a qualification image, not the
production launcher or multi-user-thread runtime. Persistent receiver-policy
authority, broad lifecycle/registry/stream/child and foreign-proxy paths,
resource qualification, the remaining guest proofs, and final artifact
qualification remain open. This is a live ledger, not a completion claim.

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
  app-instance, process-group, and window metadata. Limits are 16 app instances,
  four process members/group, and 32 ordinary windows. Handles are generation
  checked; primary/helper membership records exact manager cap-slot indices;
  owner checks and teardown are explicit. This model does not spawn, retain
  capabilities, allocate surfaces, or authorize broker requests.
- Five lifecycle host tests cover stale/reused references, helper membership,
  owner-only window close, teardown, the 16/32 target, and mutation-free
  refusal. Desktop and kernel remain unchanged; there is no guest proof.

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

- Accepted ADR-0083 freezes a userspace-owned startup-page ABI without changing
  legacy spawn: one read-only `SharedRegion` in child slot 0, existing five-cap
  spawn maximum (four additional explicit descriptors), exact one-page/RSP
  contract, and no ambient argv/env/CWD grants.
- Added `startup.rs` encoder/parser for the canonical 128-byte ARST header,
  bounded arg/env tables, exact cap kind/rights/role references, CWD and
  optional stdio descriptors, entry/load-base/page/clock facts, zero reserved
  and tail bytes, and a required one-page read-only transport cap. Five Rust
  tests and five independent Python oracle tests cover roundtrip, malformed
  offsets/padding/identity, descriptor mismatch, capacity refusal,
  startup-cap geometry, and the runtime guest vectors. The shared 4 KiB pages
  are reproducible and byte-identical across codecs. Names and descriptor
  slots remain descriptive.
- Added no_std `userspace/arena-runtime`: it validates slot 0 as the exact
  one-page `SharedRegion/READ|DESTROY`, maps read-only, copies to private fixed
  BSS, unmaps/destroys the transport before application entry, parses without
  heap/initial-stack allocation, and compares each actual child cap kind and
  rights while ignoring descriptive object IDs. Host runtime tests pass 4/4;
  Clippy is clean and the independently linked proof image builds for
  `x86_64-unknown-none`.
- `tools/test_m12_startup.py` boots the exact artifact. The guest checks
  argv/env, actual Notification slot 1, entry/base, initial RSP, IF/DF, and
  successful startup-cap reclamation. RED cases for listed-cap rights, slot-0
  rights, wrong page count, and bad magic refuse before the application
  closure. Per-case frame/process/notification/map/SharedRegion snapshots
  return exactly to baseline: m12 PASS 6/6 (four startup RED controls plus the
  reserved heap-slot collision); m1–m7 and m11 regression markers pass in the
  same boot. The proof image adds one explicitly bounded boot fixture only;
  process/cap/resource limits are unchanged.
- This is a qualification guest, not a production app manager. Installed
  registry/launcher integration, general VM, user-thread lifecycle, and the
  broad runtime remain open; the basic FS-base TLS and bounded-heap proofs are
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
  timer retires. Eight host runtime tests pass, including the frozen TCB
  layout, heap capacity/OOM, zero-on-new-page, alignment, split/coalesce/reuse
  and invalid layouts. Clippy and the `x86_64-unknown-none` guest build pass.
  TLS is not yet tested across two user threads, and heap mappings remain
  per-thread in syscall pointer validation. Process-wide user-region ownership,
  VM semantics and actual user-thread creation remain open.

## Qualification attempts and remaining gates

| Gate | Result |
|---|---|
| Phase-11 baseline guest `tools/test_m11_wm.py` | PASS; baseline only, not a Phase-12 application proof |
| APB1 independent Python/OpenSSL oracle | PASS 6/6 |
| APB1 + policy + lifecycle + AFS2 install/registry/startup Rust host tests | PASS 34/34 |
| APB1 clippy / format / no_std target build | PASS |
| Native startup ABI v2 Rust codec / independent Python oracle | PASS 5/5 + 5/5 |
| arena-runtime startup/live-cap/heap/TLS host tests / no_std build | PASS 8/8; Clippy clean; guest image builds |
| Independent m12 ring-3 startup + FS-base TLS + 32-page heap, startup/TLS REDs and heap-slot collision | PASS 6/6; exact frame/map/cap/timer/resource return; m1–m7 + m11 regression PASS |
| APKG v1 independent host tests | PASS 6/6; no wire reinterpretation |
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
   compositor. Design independent per-window publication without violating
   the 32-region, 20,480-page, 64-cap, notification, page-table, or frame
   budgets; qualify 800×600 and 1024×768 on the exact guest.
4. Continue through the remaining broad native runtime/heap/VM/TLS, threading,
   explicit helper groups, streams, userspace handles, PIE feasibility, and
   separately selected foreign proxy/ProcessMemory gates in `PLAN.md`.
5. Only after every gate, run the historical suite, artifact-bound 100/100
   stability loop, and extracted archive boot; publish hashes, logs and final
   receipts. Phase 12 is not complete.
