# Phase 12 — application-platform architecture audit

**Audited source:** `a077ee4a929a0271a28308ac2c7bb28fd2597aae` (qualified
Phase-11 archive tip, checked out as `HEAD` when this audit began).
**Qualified engineering source:** `fd7f481b46df6288708e14c5f898f6f00a3d6b02`.
**Scope:** facts below are from the checked-out source and accepted ADRs, not
from the Phase-12 request. The audit distinguishes product bounds from
mechanism bounds and records known documentation/code discrepancies rather
than silently choosing one.

## Executive finding

ArenaOS has a capable userspace desktop and sound capability primitives, but
its current application unit is effectively **one spawned process, one
reserved graphical session, one window**. The kernel has no application or
window concept. The desktop broker implements the association in userspace.
The six built-in apps and a small, separately bounded signed-Image path are
still the only launch vocabulary. The process/spawn/capability/SharedRegion
budgets were measured for twelve such sessions; they are not latent general
application-platform capacity.

The design direction is compatible with ArenaOS's accepted capability
architecture. No evidence requires putting application names, app lifecycle,
window policy, integer descriptors, POSIX, or Linux syscall semantics in the
kernel. There is, however, no safe implementation path to the requested scale
by changing `MAX_WINDOWS` alone: process, spawn-record, notification,
cap-space, SharedRegion-count, SharedRegion-page, and dynamic-Image limits
interlock.

## Source and accepted decisions inspected

Primary documents: `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, and
`docs/phase11/FINAL-REPORT.md`. Relevant accepted records: ADR-0002,
0004, 0006, 0014–0019, 0021, 0029, 0037–0049, 0052–0055, 0056–0060,
0064–0069, and 0071–0079; especially ADR-0014 (processes), ADR-0015
(capability spaces), ADR-0016 (ELF subset), ADR-0019 (spawn), ADR-0053/54/55
(signed packages/installation/Image authority), ADR-0064 (dynamic child
bound), ADR-0065/66 (desktop grants and graphical launch), ADR-0075 (measured
surface/session budget), ADR-0076/77 (AFS2/filesd), and ADR-0078/79 (desktop
and watches).

Code read: `kernel/kernel/src/{proc,sched,spawn,elf,cap,image_registry,shared,ipc}.rs`,
`kernel/kernel/src/arch/x86_64/{syscall,paging}.rs`, `userspace/abi.rs`,
`userspace/arena-lib`, `userspace/servicemgr`, `userspace/packaged`,
`userspace/package.rs`, `userspace/desktop/src/{apps,model,shell,service_wire}.rs`,
`userspace/desktop/src/bin/{desktop,application}.rs`, `userspace/filesd`,
`userspace/afs2`, and the Phase-11 tests/qualification records.

## Existing concepts and bounds

| Question | Checked-out implementation | Consequence |
|---|---|---|
| Is an application a process? | No kernel `Application` object exists. The desktop's `Session` stores one Process-cap slot, one backing SharedRegion, one client snapshot, and one window handle. Built-in sessions are launched as separate children. | Process identity is presently used as the desktop-session/lifecycle unit, but this is userspace convention, not a kernel invariant. There is no process-group/app-instance object. |
| Is an application one window? | In practice yes. `desktop::State::create` refuses a second window with the same `backing`; the broker `Session` has one `handle` and one surface/snapshot. | Multi-window-per-process requires an explicit surface/window protocol and accounting redesign, not only a larger window array. |
| Is a session a process? | In the desktop broker, yes: one `Session` is bound to the exact Process capability returned by `SYS_SPAWN`. Every launch first reserves graphical resources, even if the client never creates a window. | A headless background process is not a first-class desktop lifecycle state. The kernel itself does not know what a session is. |
| What applications exist? | `userspace/desktop/src/apps/mod.rs` declares six numeric built-in kinds: Terminal, Files, Editor, Settings, Monitor, Gallery. The service wire rejects kinds above 5; F1–F6 and the dock enumerate those six. Generic signed-image launches are separate, but do not create a persistent app catalogue. | App kind, launch descriptor, running-state/dock accounting, and some policy still use fixed six-element arrays. The service-manager manifest describes services, not installed user applications. |
| Window/session ceiling? | `desktop::model::MAX_WINDOWS = 12`; `desktop` binary's `LIMIT` uses it for its `SESSIONS` and private-clock arrays. The state also permits only one window per backing. Guest proof and ADR-0075 qualify twelve sessions/windows and explicit thirteenth refusal. | Current supported ordinary-window ceiling is 12; it is a measured Phase-11 product bound. |
| Process/spawn/thread ceilings? | `proc::MAX_PROCESSES = 32`; `sched::MAX_THREADS = 64`; each kernel thread has an 8-frame (32 KiB) kernel stack. `spawn::MAX_SPAWN_RECS = 32`; exited children retain records until Process-cap finish/reap. Phase-11 budgeting states 14 boot processes plus 12 desktop sessions = 26 records/processes before test/temporary children. | With all 12 app sessions live, only six process/record slots remain under those fixed bounds. Threads are currently one per spawned process in the public lifecycle. |
| Dynamic Images / dynamic child limit? | `image_registry::{SLOTS=2,MAX_BYTES=4096}`: two immutable kernel-owned Image snapshots, each at most 4 KiB. Dynamic ELF admission separately limits load segments to 16 pages (plus one stack page). `spawn::MAX_DYNAMIC_CHILDREN=4` counts unretired dynamic child records system-wide, including exited/unreaped children. Boot-embedded images are a separate namespace. | The signed dynamic-image path is a package test/early installer substrate, not a scalable app store. |
| Is `MAX_IMAGES` authoritative? | `spawn::MAX_IMAGES=24` has a stale comment; `image_bytes` maps static IDs 0–26, and `MAX_BOOT_IMAGES=10` bounds a distinct BootImage namespace. Dynamic Images use `image_registry::SLOTS`, not `MAX_IMAGES`. | Do not use the old constant/comment as an inventory or safety bound. Correct it when the image-source model changes. |
| Cap-table budget? | `cap::CAP_SLOTS=64` per process. Phase-11 desktop cap high-water is documented as 61 at twelve sessions (one later receipt spells it `24 + 3×12 + 1`; an earlier settled receipt is 60=`23 + 3×12 + 1`). | A larger number of sessions exceeds the broker's 64-slot space unless lifetime/reference structure or measured capacity changes. This is not kernel authority tied to app names. |
| IPC endpoints/notifications? | `ipc::MAX_ENDPOINTS=16`, `MAX_NOTIFS=31`, per-endpoint queue depth 16. ADR-0075 documents twelve private app clocks plus production notifications; 31 is full at the desktop profile. | Launching more session-scoped clients needs a clock/event allocation model and a live capacity proof. A constant-only increase can collide with fixed boot service objects. |
| Shared regions/maps? | `shared::{MAX_REGIONS=32,MAX_PAGES=1024,TOTAL_PAGES=20480,MAX_MAPS=64}`. A desktop session owns one writable client region and one private broker snapshot region; scanout is another region. | Region slots alone cap this topology at fewer than 16 full sessions before other users. `TOTAL_PAGES` is 80 MiB, and not all pages are available to the desktop. |
| Measured per-session surface cost? | Code formula at 800×600: work area 800×518; main surface `ceil(800×518×4/4096)=405` pages; transient 64 pages; one I/O page and one filesd page. Shared region = 471 pages, private snapshot = 469 pages, total = 940 pages/session (~3.67 MiB). Twelve sessions = 11,280 pages (~44.1 MiB), plus the 469-page scanout. The 2026-10-06 live `tools/test_m11_wm.py` receipt logs 471/469 and exact baseline/peak counters; see `BASELINE-RECEIPT.md`. | Reconciled for current source: the live 471/469 agrees with code. ADR-0075 and Phase-11 progress retain their historical 470/469 observation unchanged. Full physical-frame inventory and refusal-frame stability still need work. |
| ELF constraints? | `elf::validate` accepts only ELF64 little-endian `ET_EXEC`, x86-64, static/no `PT_INTERP`, at most eight program headers/accepted PT_LOAD segments, page-aligned canonical-lower-half segments, non-overlap, W^X, in-file segment data, and entry in an executable segment. No ET_DYN, relocations, dynamic linker, or ASLR. Embedded image file lengths are build artifacts; dynamic Image byte cap is 4096. | Native PIE must be an additive, separately tested loader path; existing ET_EXEC remains the compatibility baseline. |
| Startup register/stack contract? | `spawn::prepare` derives one 4 KiB RW+NX stack page immediately above the highest segment. `enter_user(entry, stack_top)` enters by `iretq`, with `RFLAGS.IF=1`; stack top is page/16-byte aligned. No return address, argc/argv/env, aux block, CWD capability, standard streams, or stable startup register initialization is supplied. The first user instruction runs with no documented entry-block contract. | Current binaries rely on bespoke entry code and/or the desktop bootstrap IPC. There is no native startup ABI v2 and no hidden libc expectation. |
| TLS? | Syscall entry documents user `GS.base=0`; no userspace FS/GS TLS setup, TCB ownership, TLS image, or per-thread errno/runtime state exists. | Any TLS feature must explicitly reconcile `swapgs`, FS/GS MSRs, context switching, and foreign runtimes. |
| User threads? | No public thread-create/join/wait syscall. Kernel `sched::spawn_in_proc` is a kernel-internal mechanism. Spawn makes one user thread per process; `SYS_THREAD_EXIT` terminates it. Kernel-side thread table is 64. | Ordinary apps are single-user-thread. No TLS or synchronization library can safely assume otherwise. |
| VM/heap? | `SYS_ALLOC_FRAME` issues one owned Untyped frame cap. `SYS_MAP_MEMORY` consumes/maps it into the calling process at a kernel-chosen VA, NX and non-executable for Untyped; it appends a user-pointer validation region. No general anonymous reserve/commit, exact unmap for ordinary Untyped mappings, protection-change, or guard-page syscall is exposed. SharedRegions have bounded create/map/unmap. Kernel heap exists; no general application `GlobalAlloc`/Arena runtime heap is linked. | Native heap/VM must be built on explicit memory authority and cleaned/accounted at process teardown. The existing SharedRegion mechanism is not an anonymous VM API. |
| Standard streams/pipes? | `SYS_DEBUG_WRITE` writes to the kernel diagnostic console (bounded, not an application stream); `SYS_CONSOLE_READ` is a global line-oriented kernel console reader; other I/O is capability IPC and SharedRegion pages. No generic byte stream, stdin/out/err endpoints, pipe, EOF, or backpressure primitive exists. | Console syscalls cannot serve as the universal app or compatibility stream API. |
| Native descriptor/handle abstraction? | Kernel cap slots are native authority. `arena-lib` wraps selected syscalls/IPC/AFS1/network calls. No optional userspace integer-handle table exists. | Any integer descriptors must be process-local runtime bookkeeping over held capabilities, never kernel fd semantics. |
| Installed app registry/launcher? | None. Six app kinds are compiled into desktop arrays and protocol discriminants. File open-by-type special-cases text/Editor (`ADR-0078`); preferences persist theme/motion and desktop icon positions, not installed app definitions, pin state, or default associations. | All Applications, search, favorites/running dock entries, and associations need registry-backed userspace policy. |
| Package shape/limit? | APKG v1 (`userspace/package.rs`) is a 128-byte manifest, one opaque payload of 1–4096 bytes, and one 64-byte Ed25519 signature; maximum package file 4,288 bytes. Payload SHA-256 is checked. `packaged` stages/installs the Phase-8 single-package AFS1 records; AFS2 supports much larger ordinary files (16 MiB tested) but has no signed multi-file bundle installer. | APKG v1 must stay unambiguous and tested. A new versioned bundle must bind a canonical manifest and every file hash, then install to AFS2 transactionally. |
| Foreign ABI/personality? | None. `syscall_dispatch` dispatches every syscall number through ArenaOS's native table. No process personality, proxy endpoint, ProcessMemory capability, or syscall-request userspace server exists. | Phase 13 must be a userspace semantics layer; the kernel change in Phase 12 should be a generic, capability-bound proxy transport only. |

## Security and scaling implications

1. **Application ID, package ID, display name, PID, and window handles are
   descriptive identities, not launch or document authority.** Current native
   spawn authority is a held `Image/READ` or `BootImage/READ` cap. Current
   desktop operations are gated by held server/region/Process/file caps plus
   broker policy. Future registry lookup must resolve to an actually verified
   immutable install and then a held executable capability.
2. **A process is not an app instance.** A future userspace app manager must
   own a generation-safe instance record and an explicit set of Process caps;
   helper launch must use an allowlist resolved to verified helper Images and
   an explicit attenuated capability list. A process exit must not destroy the
   identity of a surviving sibling process unless app policy says so.
3. **A window is not an app identity.** A compositor window must be authorized
   by an explicit surface/backing capability and exact live owner relationship.
   Multiple windows must have independent surface content or explicit
   non-overlapping sub-surfaces, with bounded publication snapshots.
4. **Scale is multi-resource.** At 32 independent full-work-area windows, the
   current one-region-plus-one-snapshot topology would require 64 session
   regions (plus scanout), roughly 30,080 shared pages at the 800×600 code
   formula (plus scanout), and about 120 MiB of session backing/snapshots. It
   also exceeds the 32-region registry, 20,480-page registry, 64-slot broker
   cap table, and 31-notification table. Therefore a useful 32-window design
   needs an intentional per-window variable/lazy reservation or equivalent
   proof, explicit broker/session lifecycle accounting, and independent table
   budgets. Raising only `MAX_WINDOWS` would create inconsistent arrays or
   mutate unrelated gates without meeting the actual target.
5. **Process/record headroom is similarly coupled.** The 14+12 qualified
   baseline leaves 6 of 32 process/record entries. A 16-instance installed
   app target can fit only if system-process inventory, app child bounds,
   exited-record retirement, per-app window surfaces, and cap/notification
   usage are budgeted and proven together.

## Audit conclusions

- No fundamental conflict with ADR-0002/0006 was found. Keep app discovery,
  lifecycle policy, associations, descriptor translation, and foreign syscall
  meanings in userspace.
- The first architectural change should be a Phase-12 umbrella ADR, followed
  by individually testable format/runtime/proxy decisions. Do not increase
  capacities until the relevant working set and high-water are measured.
- The accepted Phase-11 properties remain constraints: exact capability
  authority, fail-closed AFS2/filesd, generation-safe identities, explicit
  spawn inheritance, W^X/NX, bounded IPC, and historical test preservation.

## Phase-12 host slices added after the audit

`userspace/arena-platform/` now contains a host-testable no_std lifecycle
metadata model (16 app instances, four process members per group, 32 window
records), a manifest/catalog model, and a bounded APB1 verifier. It also has
an AFS2-engine installer core: private staging, `APB1.record` metadata/signature
persistence, 4 KiB file streaming, durable readback verification, immutable
version-path collision refusal, atomic AFS2 rename activation, and fixed-depth
staging cleanup. The engine is reused through a path dependency on the already
accepted `userspace/afs2` implementation.

These additions are userspace library and host-proof work only. They do not
modify the desktop broker's 12-slot `Session`/one-window-per-backing topology,
filesd authority records, kernel resource limits, launch Image policy, or the
Phase-11 guest behavior. The host policy-aware API reuses APKG `Chain` rules
and binds the verified claim/digest, but persistent chain loading remains a
trusted caller responsibility; the low-level installer also accepts an
explicit key for testing/adapters. Both paths receive directory object IDs
inside an already-authorized host `Volume` and hold no capabilities. No guest
service invokes them yet. Therefore the audit findings and capacity
constraints above remain current for production behavior; do not count the
host model's 16/32 metadata bounds as measured system capacity.
