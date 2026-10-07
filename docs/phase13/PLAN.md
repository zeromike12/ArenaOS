# Phase 13 — Native Runtime & Desktop Application Maturity

**Branch:** `arena/phase13-native-app-maturity`
**Parent:** `330a691797343c8ef997cbb791ca54ec10e89fc5`
**Qualified Phase-12 implementation ancestor:** `cd8c78a0189ce365fd5af93b0006f15fb647ed6f`

## Goal and invariants

Turn the qualified Phase-12 native foundations into a package-backed application
platform. Installed application metadata, `AppInstance`, `ProcessGroup`,
processes, helpers, and windows remain separate. Only possession of exact live
capabilities authorizes launch, document access, process control, and service
use. Preserve Phase-12 APB1 receiver verification, AFS2 protected installation,
the 128-slot cap inventory and observable slot 127, low-32 manager accounting,
the ABI-v2 startup gate, and all qualified historical behavior. Linux/POSIX
semantics and visual redesign are out of scope.

## Audited Phase-12 production path

The starting path is:

1. Desktop offers a selected `.apb1` source as an exact read-only filesd File
   capability to `packaged`.
2. `packaged` selects a signer through the APKG-v1 policy chain, asks filesd
   to inspect and verify APB1, then asks filesd to install it.
3. filesd validates the internal install badge, re-verifies the source,
   stages bounded chunks below its private AFS2 staging root, verifies durable
   readback, and atomically activates the immutable
   `/System/Applications/<app-id>/<version>` tree.
4. There is no production boot registry scan, no installed-application Image
   resolver, and no installed-app launch operation. The reusable
   `rebuild_installed_registry` implementation in `arena-platform` is not
   called by a boot service; a successful install is not launch authority.
5. Desktop's production launcher rejects kinds above 5 and selects one of six
   built-in launch profiles. ABI-v2 launch creates one broker session, one
   Process child, one surface, one ordinary window, and the exact startup cap
   list. Desktop offers document authority only through its current Editor
   path. App/window/process tables and APB1 metadata APIs exist as foundations,
   but do not yet form a package-backed app manager.

## Initial resource inventory

Bounds below are source constants and existing ABI limits; Phase 13 will not
raise them until an end-to-end budget justifies each change.

| Resource | Current bound / state | Source of bound |
|---|---:|---|
| Processes | 64 | `kernel/kernel/src/proc.rs` |
| Scheduler threads | 64 global; no user-thread creation API | `kernel/kernel/src/sched/mod.rs` |
| Spawn records | 64 | `kernel/kernel/src/spawn.rs` |
| App instances | 32 | `arena-platform::lifecycle` / ABI startup slots |
| Ordinary Desktop windows | 32 | `desktop::model`; one built-in process/window per session today |
| ProcessGroup members | 4 per group | `arena-platform::lifecycle` |
| SharedRegions | 80 records; up to 1,024 pages each; 36,864 aggregate pages | `kernel/kernel/src/shared.rs`, ADR-0088 |
| User mappings | 128 shared maps; 80 user pointer regions per thread | `shared.rs`, `sched/mod.rs` |
| Capability slots | exactly 128 per process; slot 127 separately observable | `cap.rs`, ADR-0086/0088/0091 |
| Endpoints | 16 | `kernel/kernel/src/ipc.rs` |
| Notifications | 64 | `ipc.rs` |
| IPC queue | depth 32 per endpoint | `ipc.rs`, ADR-0089 |
| Timers | 32 global, 4 per process | `kernel/kernel/src/timer.rs` |
| Native heap | 32 lazily committed 4 KiB pages; slot 63 transient allocator cap | `arena-runtime::heap`, ADR-0084 |
| Startup inherited capabilities | at most 5; exact ABI-v2 inventory | `spawn.rs`, startup ABI, ADR-0083/0086 |
| Dynamic Image registry | 2 live entries, 4 KiB per Image | `kernel/kernel/src/image_registry.rs` |
| Installed app catalog | 64 descriptive records; production boot integration absent | `arena-platform::registry` |
| Native byte streams | not implemented | startup ABI stream roles are reserved but absent |

The process owns the PML4 and capability space, but syscall user-range records
are currently held on each scheduler thread. `SYS_MAP_MEMORY` installs owned
RAM as writable/NX or MMIO as uncached/NX and appends to the calling thread's
region table. Ordinary mapped pages have no release/protection operation;
process teardown reclaims the address space. This mapping model cannot safely
support multiple user threads sharing heap and syscall buffers.

## Current implementation state

## Current native resource additions

The following are Phase-13 bounds layered on the unchanged Phase-12 inventory:

| Resource | Current bound / behavior | Source |
|---|---:|---|
| Process-owned VM regions | 128 global; 8 per process; 4,096 pages (16 MiB) per region | kernel::vm, ADR-0095 |
| Committed native VM pages | 8,192 per process; 32,768 global | kernel::vm |
| Commit/protect operation | at most 64 pages; refusal leaves its PTE and accounting set unchanged | kernel::vm |
| Process user spans | 80 per process, shared by all its threads; each VM region occupies one span including both guards | ADR-0094/0095 |
| Scalable application heap | 4,096 pages (16 MiB) reserved lazily; physical frames committed on allocation | arena-runtime::heap, ADR-0096 |
| Scalable heap alignment | ordinary block alignment 16 bytes; page-run alignment up to 2 MiB | ADR-0096 |
| Scalable heap committed backing | retained for reuse after deallocation; exact VM region release or process teardown returns frames | ADR-0095/0096 |
| Process exit status | stable full-width final status until reap; exact Process/READ required | syscall 61, ADR-0100 |

The Phase-12 BoundedHeap, its 32-page limit, and startup proof remain intact.
The Phase-13 application opts into ScalableHeap; no reserved capability slot
is borrowed as a transient frame holder.

The live Desktop still holds at most 32 application Process sessions and the
window policy still holds at most 32 total ordinary windows. One session can
now own several windows; an additional window consumes one fresh SharedRegion,
one private snapshot SharedRegion, two Desktop mappings, a child mapping, and
one exact transferred cap while live. The extra-window record table is
bounded at 32, but the Window Manager remains the authoritative total limit.
Process-owned user spans remain 80 per process and are shared by all its
threads.

The initial audit above describes the branch at its starting SHA. The first
implementation checkpoint now integrates protected installed APB1 enumeration
and re-verification through filesd, the packaged receiver-policy catalog, an
exact dynamic Image-capability launch path, and an initial All Applications
surface. A real QEMU guest installed a signed bundle, launched the verified
ELF twice through keyboard/search and pointer selection, observed two ABI-v2
processes and compositor windows, and returned identity resource counts to
baseline after close. See `PROGRESS.md` and ADR-0093 for measured details.

The launcher has registry-backed associations and an AFS2-persisted Open With
flow. Its T1 guest proof verifies that a saved default creates no authority
and that an explicitly selected installed handler receives the exact
read-only File capability. ADR-0097 and the signed APB1 T1 guest now prove
three independently backed ordinary windows under one process and two such
instances at once. ADR-0098 adds a signed installed headless launch with an
exact Process cap, one attenuated Notification, no window or Desktop endpoint,
and real timer-driven teardown. Persisted launcher favorites, helper
processes, streams, user threads,
synchronization, mixed-load pressure, and final qualification remain open. The
dynamic Image envelope is a bounded executable staging mechanism; it is not
process VM and does not imply PIE or ASLR support. See `PROGRESS.md` for
measured receipts.

## Work plan

### A. Package registry and executable authority

- Wire a trusted boot owner to enumerate protected APB1 installations, verify
  current receiver policy and exact installed trees, atomically publish a
  bounded descriptive registry, and refresh it after install/replacement.
- Define an installed executable contract and resolve each descriptive record
  through the verified immutable version to a live exact Image capability.
- Reject guessed IDs, stale versions, malformed trees, app-ID collisions, and
  any executable whose bytes do not match the verified APB1 record.
- Keep APKG v1 staging/activation and APB1 bundle installation policy
  distinct.

### B. Launcher and documents

- Make All Applications enumerate the verified registry, support bounded
  search/filter and keyboard/pointer selection, and launch installed apps.
- Track running instances independently from installed definitions; derive
  dock entries from persisted favorites plus live instances.
- Replace Editor-only open behavior with content-type handler lookup, persisted
  defaults, and Open With. Only the broker may explicitly lend the selected
  exact File capability; cancellation or metadata selection grants nothing.
  **Status:** handler filtering, persisted defaults, cancel cleanup, and
  exact read-only installed-app handoff are implemented and guest-proved;
  built-in Editor behavior remains available through its declared types.

### C. Instance, process, and window lifecycle

- Connect `AppInstanceTable`, exact Process-capability groups, and a separate
  window set to production broker state. **Status:** each occupied Desktop
  app-instance slot owns a separate four-member `ProcessGroup`; generation-
  safe group handles wrap exact Process caps, and retirement drains the whole
  group before clearing its owner slot (ADR-0099). The reusable
  `AppInstanceTable`/`WindowSet` remains a host-side model; the Desktop session
  record still needs a complete multi-process instance lifecycle.
- Give each ordinary window independent surface, compositor, damage,
  publication, close/state, and accounting ownership; prove one app owns at
  least three simultaneous windows. **Status:** window-level production
  ownership and the three-window installed APB1 guest proof pass (ADR-0097);
  per-instance ProcessGroup ownership is guest-regressed (ADR-0099); explicit
  helper launch and lifecycle remain open.
- **Status:** a signed headless primary launch with explicit attenuated
  Notification authority and exact Process-cap wait/reap is implemented and
  guest-proved (ADR-0098). Stable final exit status is queryable and waitable
  through the member's exact Process capability (ADR-0100). Add signed
  allowlisted helpers with explicit grants, terminate policy, crash handling,
  and manager-death policy.

### D. Streams

- Add a bounded native byte-stream service or shared-ring/notification
  implementation with endpoint authority, partial transfer, backpressure,
  event notification, EOF, close, endpoint death, and teardown.
- Connect Startup ABI-v2 stdin/stdout/stderr roles to those exact stream caps.

### E. Process-wide VM, heap, threads, synchronization

- Specify process-owned mapping metadata and minimal reserve/map/release/protect
  operations, including atomic admission, W^X, guard pages, device-memory
  rejection, accounting, and teardown. **Status:** process-owned pointer
  validation and exact-cap VM reserve/commit/protect/release/query are
  implemented (ADR-0094/0095). M12 startup, 32-session, and installed-app
  guest regressions passed, followed by 20/20 clean boots on that checkpoint's
  exact EFI.
- Replace the 32-page heap ceiling with lazy, bounded, fallible process VM
  backing and observable allocator/page accounting.
- Only after process-wide mapping ownership is implemented, add real user
  threads with explicit guarded stacks, per-thread FS.base TLS, join/exit, and
  bounded process cleanup. Add native Mutex, Condvar/wait-notify, and Once
  semantics without Linux futex behavior.

### F. Qualification and handoff

- Make a representative mixed workload with at least 16 live app instances,
  32 ordinary windows, one three-window app, installed APB1 apps, helpers,
  threads, file caps, streams, timers, and notifications.
- Measure each identity-bearing resource at baseline, high-water, churn, and
  full teardown; explain retained page tables and prove refusal atomicity.
- Audit ET_DYN/PIE and ASLR. Implement only a small statically linked native
  subset if it fits without destabilizing the platform; otherwise record the
  exact blocker and next phase.
- Run feature-level guest proofs, then affected regression sets and the final
  Phase-13 historical, guest, stability, exact-artifact, and extracted-archive
  qualification.
- Produce `FINAL-REPORT.md` and `PHASE14-HANDOFF.md` only after the source and
  qualification receipts are complete.

## Regression dependency map

| Changed subsystem | Required development regressions |
|---|---|
| APB1, installed-tree scan, policy, filesd | APB1/APKG format and install; AFS2; filesd; packaged; registry/launch authority guest |
| Desktop registry, launcher, associations | Desktop M10/M11; Files and document authority; APB1 install/launch; UI model tests |
| App/process/window lifecycle | compositor and Desktop; M10/M11; Phase-12 M12 32-session and resource tests |
| Streams | stream model and real byte-transfer guest; IPC cancellation/server death; process teardown |
| VM and heap | allocator/startup M12; scheduler; spawn/teardown; IPC; SharedRegion; M8.5/M9 resources |
| Threads and synchronization | scheduler/context switching; TLS; mapping ownership; process death; IPC waits/notifications/timers; concurrent guest tests |
| Final artifact or packaging only | exact EFI/archive build and independent extracted boot as applicable |
