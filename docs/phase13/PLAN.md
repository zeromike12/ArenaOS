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

### Phase-12 behavior hard-coded to the six built-in applications

The starting hard-coded path was concrete and limited to these owners:

- Desktop dispatch accepted built-in kinds 0–5, chose the matching static
  boot Image and startup policy, and made one process/window per launch.
- All Applications did not exist as an installed catalog surface; the dock's
  six built-in tiles were the entire visible launch inventory.
- Document open selected Editor kind 2. There was no signed content-type
  handler list or AFS2 user default.
- Desktop sessions were one-window records; the reusable `AppInstanceTable`
  did not own production process or window state.
- Packaged APKG v1 dynamic Images were limited to two 4 KiB images and four
  dynamic child processes. There was no APB1 installed Image resolver.
- Standard-stream roles, helper membership, user-created threads, process VM
  regions, and native synchronization objects had no production lifecycle.

These are the exact Phase-12 demo ties removed or extended in production during
Phase 13. The six built-ins remain as applications and regression fixtures;
they no longer define the entire application inventory or launch mechanism.

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

The notification pool remains exactly 64. The full 32-session boot inventory
uses 51, leaving at most 13 dynamic Notification objects at runtime. Helper
timer requests consume this existing bound and are refused when it is full;
the manager retains one owner cap per live helper timer and retires it only
after that helper's exact Process cap is reaped.

The helper fixture performs 15 sequential signed helper launches, including
crash/reap and AppInstance cleanup. It therefore creates more private timer
objects over the test than the 13-object dynamic headroom and proves the
objects are reclaimed as each exact Process capability is retired.

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
| SharedRegion records / maps | 96 / 160 bounded live records; aggregate page budget remains 36,864 | ADR-0104; sized for 16 stream-enabled instances and 32 windows |
| Native standard streams | one page per opted-in instance or explicitly streamed helper; three single-producer/single-consumer rings, 768 bytes each | ADR-0104/0105 |
| User-created ring-3 threads | at most 4 additional threads per Process; 64 global scheduler slots unchanged | ADR-0106 |
| Native synchronization | 64 capability-owned domains, 32 keys/domain and 2,048 global keys, 64 parked waiters | ADR-0107 |
| User-thread stacks | one exact 16-page VmRegion per thread; one metadata/TLS page, one uncommitted guard, 14 committed RW/NX pages | ADR-0106 |
| Thread lifecycle | same-process stable status join or detach; kernel stack and detached VmRegion cleanup after the thread is switched off | ADR-0106 |
| Spawn grants / startup descriptors | at most 8 grants (including slot-0 startup cap) / 7 descriptors; 128 process capability slots and slot-127 accounting are unchanged | ADR-0104 |
| Installed application metadata | 64 boot-rebuilt descriptive entries; each launch rechecks the verified installed tree and obtains a fresh exact Image cap | ADR-0092/0093 |
| Installed native Images | 16 live kernel Image records, at most 256 KiB per Image and 128 mapped PT_LOAD pages; 24 dynamic child slots | ADR-0093 |
| ProcessGroups | 32 AppInstances, up to 4 exact Process-cap members per group | ADR-0099/0101 |
| Ordinary windows | 32 globally; a Session/AppInstance can own multiple independently backed windows | ADR-0097 |
| Package handler/favorite preferences | bounded AFS2 records; application IDs select metadata only, stale registry references are pruned | `desktop::favorites`, `desktop::associations` |

The Phase-12 BoundedHeap, its 32-page limit, and startup proof remain intact.
The Phase-13 application opts into ScalableHeap; no reserved capability slot
is borrowed as a transient frame holder.

The live Desktop still holds at most 32 application Process sessions and the
window policy still holds at most 32 total ordinary windows. One session can
now own several windows; an additional window consumes one fresh SharedRegion,
one private snapshot SharedRegion, two Desktop mappings, a child mapping, and
one exact transferred cap while live; the instance retains its own SyncDomain
owner cap for native synchronization. The extra-window record table is
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
and real timer-driven teardown. Signed helper execution and manager cleanup
are guest-proved by ADR-0101. ADR-0104 and the registry T1 guest prove
Startup ABI standard streams, focused keyboard stdin, output backpressure,
partial transfer, EOF, and exact owner teardown. ADR-0105 extends the same
bounded rings to allowlisted helpers, with a capability returned only after
explicit request and owner-scoped wake through the authenticated manager.
AFS2-persisted favorites now feed the launcher and dynamic dock. The mixed
pressure fixture reached 26 live AppInstances and 32 ordinary windows with
five installed instances, five helpers, parked document threads, streams,
timers, and notifications; close-half, reuse, and full teardown returned
identity-bearing resources to their boot baseline. ADR-0109 records the
high-water and retained-frame receipt. ADR-0108 records the deliberate
ET_DYN/PIE deferral. The dynamic Image envelope is a bounded executable
staging mechanism; it is not process VM and does not imply PIE or ASLR support.
See `PROGRESS.md` for measured receipts and current qualification state.

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
  **Status:** boot-time filesd enumeration rebuilds a bounded catalog only
  from protected installed records, re-verifies APB1 bytes and receiver policy,
  and publishes descriptive entries. Launch resolves the exact installed
  version into a kernel-validated Image capability; stale versions, malformed
  packages, ID collisions, and metadata-only guesses do not resolve to that
  authority (ADR-0092/0093).

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
  window set to production broker state. **Status:** each generation-bearing
  Desktop Session is a distinct AppInstance record and owns a separate
  four-member `ProcessGroup`; the group tracks the primary process and
  explicitly allowlisted helpers through exact Process caps. Retirement drains
  the group before clearing the owner slot (ADR-0099/0101). Window ownership
  refers to the AppInstance key, not a process or window ID. The reusable
  `AppInstanceTable`/`WindowSet` remains a host-side policy model and carries
  no production launch authority.
- Give each ordinary window independent surface, compositor, damage,
  publication, close/state, and accounting ownership; prove one app owns at
  least three simultaneous windows. **Status:** window-level production
  ownership and the three-window installed APB1 guest proof pass (ADR-0097);
  per-instance ProcessGroup ownership is guest-regressed (ADR-0099). Signed
  AHL1 helper resolution, exact Image launch, private timer notifications,
  owner-signal attenuation, wait/reap, terminate, fault status, and group
  cleanup are implemented and guest-proved (ADR-0101).
- **Status:** a signed headless primary launch with explicit attenuated
  Notification authority and exact Process-cap wait/reap is implemented and
  guest-proved (ADR-0098). Stable final exit status is queryable and waitable
  through the member's exact Process capability (ADR-0100). Signed AHL1
  allowlisted helpers receive a fresh READ|WRITE timer Notification and an
  optional distinct WRITE-only AppInstance signal. Fifteen real helper
  launches on the 13-object dynamic notification budget prove reuse after
  wait/reap and teardown; an orphan helper is stopped with its AppInstance.
  Unknown helper IDs and attempts to use the un-inherited factory are refused.
  The boot root fail-stops if Desktop dies, because no independent authority
  can reconstruct the exact groups (ADR-0103).

### D. Streams

- Implement ADR-0104's one-page SharedRegion stream set with three bounded
  rings, partial transfer, backpressure, event wake hints, EOF, peer closure,
  and owner teardown. The startup role uses the exact stream-region cap;
  manifest opt-in remains descriptive and grants no authority. **Status:**
  standard streams are implemented and the real installed-app guest proves
  ring-full refusal/resume, EOF after close, and exact authority refusal for
  stream handles used as Notifications. Startup flag/role disagreement is
  rejected (ADR-0104).
- Wire stdin to focused Desktop input and stdout/stderr to Desktop's native
  log sink. **Status:** focused inputd-decoded keyboard bytes reach the exact
  AppInstance stdin ring; app output reaches the bounded Desktop log sink;
  stream page, mappings, and caps retire with the exact ProcessGroup. Extend
  the same exact-region mechanism to explicitly allowlisted helper channels
  without ambient inheritance. **Status:** ADR-0105 and the signed registry
  guest prove one dedicated helper page, Startup ABI stream roles, owner-only
  wake requests, real parent-to-helper stdin, helper stdout, refusal for a
  non-stream helper, EOF, and exact wait/reap teardown.

### E. Process-wide VM, heap, threads, synchronization

- Specify process-owned mapping metadata and minimal reserve/map/release/protect
  operations, including atomic admission, W^X, guard pages, device-memory
  rejection, accounting, and teardown. **Status:** process-owned pointer
  validation and exact-cap VM reserve/commit/protect/release/query are
  implemented (ADR-0094/0095). M12 startup, 32-session, and installed-app
  guest regressions passed, followed by 20/20 clean boots on that checkpoint's
  exact EFI.
- Replace the 32-page heap ceiling with lazy, bounded, fallible process VM
  backing and observable allocator/page accounting. **Status:** the signed
  installed-app fixture uses a lazy 16 MiB ScalableHeap, grows by 64-page
  commits, exercises normal Rust `Vec`, reuse/coalescing, 64 KiB alignment,
  and fallible OOM, then returns VM pages at teardown (ADR-0096).
- Only after process-wide mapping ownership is implemented, add real user
  threads with explicit guarded stacks, per-thread FS.base TLS, join/exit, and
  bounded process cleanup. **Status:** implemented under ADR-0106, with the
  preservation checkpoint passed. Four
  concurrent user workers ran in each of three installed-app launches with
  shared heap and RO VM, distinct FS.base TLS, exact stack-cap refusal, join,
  detach, and VM baseline return. A killed helper had a live user worker; its
  ProcessGroup cleanup returned to baseline. The exact EFI passed 20/20
  fresh preservation boots after the user-thread implementation. Native
  Mutex, Condvar/wait-notify, Once, timeout, and teardown now pass the signed
  installed-app guest under ADR-0107. The kernel wait domain is justified by
  the existing single-waiter Notification, one-shot timer, and process IPC
  limits. Its T1 proof, built-in startup-profile regression, and 20/20 T3
  preservation checkpoint pass.

### F. Qualification and handoff

- **Status:** the real installed APB1 pressure guest reached 26 AppInstances,
  32 ordinary windows, and three windows in one AppInstance. Five installed
  app instances owned five helpers and streams; two document workers were
  parked; the run exercised timers, notifications, exact document caps,
  half-close, helper failure, relaunch, and full teardown. Identity counts
  returned to baseline. ADR-0109 records high-water and post-teardown counts.
- Measure each identity-bearing resource at baseline, high-water, churn, and
  full teardown; explain retained page tables and prove refusal atomicity.
  **Status:** the fixture's resource-detail syscall and Desktop receipt cover
  threads, endpoints, notifications, timers, process VM regions/pages,
  SyncDomains/keys/waiters, AppInstances, windows, helpers, and stream sets;
  existing manager counters cover processes, SharedRegions, pages, maps, and
  caps. Admission/refusal controls remain at their existing bounded limits.
  Peak use was 49/64 scheduler threads, 46/64 processes, 71/96 SharedRegions,
  30,555/36,864 SharedRegion pages, 119/160 maps, 119/128 Desktop capability
  slots, 12/16 endpoints, 56/64 notifications, and 8/32 timers. After close,
  all measured identity counters matched boot; 79 fewer free physical frames
  remained, consistent with retained page-table backing seen in the M12
  scale proof. See ADR-0109 for the exact snapshots and caveat.
- Audit ET_DYN/PIE and ASLR. Implement only a small statically linked native
  subset if it fits without destabilizing the platform; otherwise record the
  exact blocker and next phase. **Status:** ADR-0108 defers `ET_DYN`, relative
  relocations, and ASLR; current applications remain static non-PIE `ET_EXEC`.
- Run feature-level guest proofs, then affected regression sets and the final
  Phase-13 historical, guest, stability, exact-artifact, and extracted-archive
  qualification.
- Produce `FINAL-REPORT.md` and `PHASE14-HANDOFF.md` only after the source and
  qualification receipts are complete.

**Final result:** the Phase-13 source image at implementation checkpoint
`c4ad723a3e66b8b72a020589cd9ab6eab73de34d` passed 115/115 historical suite
groups, the signed 26-AppInstance/32-window pressure guest, all affected M9,
M10, M11 and M12 regressions, and 100/100 fresh boots of EFI SHA-256
`3afbceffc8c98e65790552589c5bd4bb59e4245ad4f3599d7cef093ff652f42d`. The
standalone `phase13-complete` archive was extracted into a fresh directory and
its bundled EFI/ESP/OVMF/AFS2 fixtures passed a separate real QEMU guest witness.
The witness proved both signed package installs, installed registry launch,
All Applications search, exact read-only document handoff, three independently
backed windows, helper and headless process lifecycle, streams, user threads,
VM/runtime synchronization, identity-resource teardown, and clean shutdown.
Exact receipts and evidence are recorded in `FINAL-REPORT.md`; the artifact
digest is intentionally recorded there instead of in the self-containing
archive report copy.

## Regression dependency map

| Changed subsystem | Required development regressions |
|---|---|
| APB1, installed-tree scan, policy, filesd | APB1/APKG format and install; AFS2; filesd; packaged; registry/launch authority guest |
| Desktop registry, launcher, associations | Desktop M10/M11; Files and document authority; APB1 install/launch; UI model tests |
| App/process/window lifecycle | compositor and Desktop; M10/M11; Phase-12 M12 32-session and resource tests |
| Streams | stream model and real byte-transfer guest; IPC cancellation/server death; process teardown |
| VM and heap | allocator/startup M12; scheduler; spawn/teardown; IPC; SharedRegion; M8.5/M9 resources |
| Threads and synchronization | scheduler/context switching; TLS; mapping ownership; process death; IPC waits/notifications/timers; native SyncDomain table and concurrent guest tests |
| Final artifact or packaging only | exact EFI/archive build and independent extracted boot as applicable |
