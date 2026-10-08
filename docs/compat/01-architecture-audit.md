# 01 — C runtime architecture audit (ArenaOS Phase-13 baseline)

Status: analysis document. Branch `arena/f89ea987-arenaos` from
`arena/phase13-native-app-maturity` @ 74ace4f. No production kernel or
userspace file was modified for this audit.

Evidence labels used throughout: **[CODE]** read from the repository,
**[GUEST]** executed under ArenaOS in QEMU (serial evidence in
`experiments/c-runtime/build/`), **[HOST]** host-only test, **[STATIC]**
checked from an ELF/disassembly without executing it, **[PROPOSAL]**
recommended but not landed.

The question this audit answers: what does a C program need from ArenaOS,
what does ArenaOS already provide, and where are the gaps? It covers the
eight areas the assignment names.

---

## 1. Startup and the ELF contract

**What the kernel does** [CODE `kernel/kernel/src/elf.rs` `validate()`,
`load()`; `spawn.rs` `prepare()`]:

- Accepts only static `ET_EXEC`, `EM_X86_64`. `ET_DYN` and `PT_INTERP` are
  refused. This is the ADR-0108 contract; Luna's Phase-14 static PIE/ASLR
  work owns any change to it.
- Requires 1–8 program headers, `PT_LOAD` segments that are page-aligned,
  `filesz <= memsz`, no W+X, no overlaps, no kernel-half addresses, and an
  entry point inside an executable segment.
- Maps only `PT_LOAD`. Every other program-header type is inert. **`PT_TLS`
  is ignored**, so nothing initializes a thread-local block for C code.
- Takes **one 4 KiB page** above the highest segment as the initial stack.
- Dynamic Image registry (`image_registry.rs`): 16 live entries, 256 KiB and
  128 load pages per Image. Dynamic-child budget 24 (ADR-0064).

**What a process receives at entry:** nothing documented beyond the stack.
The ABI-v2 Startup block (ARST v2, `userspace/arena-startup-abi`) is placed
in slot 0 of a read-only SharedRegion and is consumed by the Rust
`arena-runtime` `run()`. A C program that does not implement that protocol
receives **no argc/argv, no environment, and no startup capabilities**
through a stable contract. [CODE]

**Gap:** the C startup must (a) switch off the one-page stack, (b) install
TLS by hand, (c) run `.init_array`, (d) exit through `SYS_THREAD_EXIT`.
The prototype does all four. [GUEST: T1 entry-stack-tls PASS]

## 2. Syscall ABI

- Convention (ADR-0017): `RAX` = call number; `RDI, RSI, RDX, R10, R8, R9`
  = arguments; result in `RAX` as a signed 64-bit value (0 = OK, positive =
  payload, negative = typed status). The kernel preserves RBX, RBP and
  R12–R15; RCX and R11 are clobbered by `syscall`/`sysret`. [CODE, abi.rs]
- 71 `SYS_*` constants (numbers 1–75, with gaps) in `userspace/abi.rs`, dispatched in
  `kernel/kernel/src/arch/x86_64/syscall.rs`. The C mirror
  (`experiments/c-runtime/include/arena/abi.h`) covers 13 of them. The
  cross-check (`run.py abi`) reports 0 mismatches against both
  `userspace/abi.rs` and the kernel. **Limitation:** a constant missing from
  the kernel is accepted silently (`k is None` passes), so the check proves
  agreement with abi.rs only. [STATIC]
- **Risk F1 — unconditional serial backdoor.** `SYS_DEBUG_WRITE` (1) copies
  up to a capped length from any user process to the serial console with
  **no capability check**. Its doc comment calls it a temporary diagnostics
  backdoor (ADR-0017). Any C program can flood the console today. The
  prototype uses it for stdout only because no capability-based stream is
  exposed to C. This must not survive into a C-facing stdio contract.
  [CODE, syscall.rs `sys_debug_write`; GUEST: all probe output arrives this way]

## 3. Capabilities

- Each process has a **128-slot** capability table (`cap.rs` `CAP_SLOTS`).
  Capabilities are typed `CapObj` values with rights bits `RIGHTS_READ`,
  `RIGHTS_WRITE`, `RIGHTS_COPY` and `RIGHTS_DESTROY` (`cap.rs`). Authority is held only through exact slots;
  paths, PIDs and names grant nothing (the Phase-13 rule, preserved here).
- Relevant kinds for C: `Image` (executable authority), `SharedRegion`
  (bytes: the native stream rings), `Notification` (wake hints only),
  `Endpoint` (IPC to services such as filesd and netd), `VmRegion`,
  `Process`, `Mmio` (driver-only), `Power`.
- The C prototype's probe is spawned with **zero grants** and needs none:
  its only kernel interactions are debug write, VM reserve/commit/release,
  TLS set, clock and thread exit, none of which require a capability.
  [GUEST: probe ran with `grants=[]`]
- **Gap:** there is no C-facing wrapper for capability transfer, cap
  description, or stream attachment. A C program cannot ask for a file or a
  socket today.

## 4. Memory

- **VM regions** (`vm.rs`): `SYS_VM_RESERVE/COMMIT/PROTECT/RELEASE/QUERY`
  (numbers 56–60). Limits: 8 regions per process, 4,096 pages per region,
  64 pages per commit operation, 8,192 committed pages per process, 32,768
  globally, 1 guard page per region. `SYS_VM_PROTECT` requires WRITE on the
  VmRegion capability. Reservations are guarded and lazy (`vm.rs`). [CODE]
- The C allocator (`experiments/c-runtime/src/alloc.c`) reserves 1,024 pages
  (4 MiB) in one region and commits in 16-page chunks. It uses 16-byte headers
  and size classes from 16 B to 1 MiB, with per-class free lists and
  **no coalescing**.
  [CODE, GUEST, HOST]
- **Measured limitation:** under a fixed 200,000-operation stress run (host,
  ASan+UBSan), 10,923 allocations were refused while the heap was only
  partly live. Refusals are typed (NULL), not corruption, but fragmentation
  is real without coalescing. [HOST]
- The Rust `ScalableHeap` (16 MiB lazy reservation, per `PHASE14-HANDOFF.md`)
  and `BoundedHeap` in `userspace/arena-runtime/src/heap.rs` are the Rust
  global allocators. Neither is exposed as a C ABI. [CODE]

## 5. Threads

- `SYS_THREAD_CREATE` (64), `SYS_THREAD_JOIN` (65), `SYS_THREAD_DETACH` (66),
  `SYS_THREAD_COUNT` (67), `SYS_THREAD_YIELD` (62). Each process can add at
  most **four** ring-3 threads (`MAX_USER_THREADS_PER_PROCESS = 4`) inside a
  global 64-thread scheduler. `SYS_THREAD_CREATE` creates the context inside
  the same Process (its doc comment). [CODE sched/mod.rs, syscall.rs]
- Synchronization: `SYS_SYNC_DOMAIN_CREATE` (68) through `SYS_SYNC_INFO`
  (74), built on capability-owned SyncDomains with generation-checked keys
  and atomic sequence-and-park waits. Mutex, Condvar and Once exist in the
  Rust runtime. **No C wrappers exist yet.** The model is ArenaOS-native and
  does not follow Linux futex semantics. [CODE]
- **Gap:** the C runtime has no thread API and no C mutex. The probe is
  single-threaded.

## 6. TLS

- `SYS_TLS_SET` (53) installs FS.base for the calling thread. A non-zero base
  must be 16-byte aligned and a writable user range (`user_range_writable`).
  [CODE syscall.rs `sys_tls_set`]
- The scheduler saves and restores FS.base on every context switch
  (`sched/mod.rs` lines ~1184 and ~1243). [CODE]
- The linker emits a variant-II PT_TLS. The C prototype computes the block
  and writes the TCB self-pointer at `%fs:0` itself. **This is where the
  first guest run failed**: the linker script under-reported TLS memsz by
  the alignment padding (see report 03). The fix is verified by an audit
  rule added to `run.py`. [GUEST, STATIC]
- **Risk / proposal:** TLS is currently a runtime convention, not a loader
  feature. Supporting PT_TLS in the loader would be a loader change and
  needs its own ADR ([PROPOSAL] in ADR-0110).

## 7. Stdio and streams

- No POSIX file descriptors exist. The Phase-13 native stdin/stdout/stderr
  is a single bounded page-backed **SharedRegion** holding three 768-byte
  single-producer/single-consumer rings. Reads and writes support partial
  transfer, backpressure, EOF, peer closure and wake notifications. The
  SharedRegion cap authorizes bytes; notifications are hints only. **This is
  explicitly not a POSIX pipe ABI.** [PHASE14-HANDOFF.md]
- No C binding to those rings exists. The C prototype writes through
  `SYS_DEBUG_WRITE` (Risk F1). A proper C stdio goes through the stream
  capability and must be proposed in ADR-0110.
- `printf` in the prototype is a bounded subset (flags `-`/`0`, width,
  precision on strings, `hh/h/l/ll/z/j/t` lengths, `%% %c %s %d %i %u %x %X %o %p`).
  Unknown conversions are echoed. No floating point. [CODE, HOST]

## 8. Filesystem

- Userspace has no path namespace and no ambient root. Persistent storage is
  served by **filesd** (AFS2, ADR-0076/0077) over IPC, reached only through
  an endpoint capability. The Rust `arena-lib/src/fs.rs` is the client.
  [CODE]
- In this boot the AFS2 region is absent (`filesd: no AFS2 region (disk
  smaller than 72 MiB)`), and the desktop reports `[desktop] AFS2 file service
  unavailable; file capabilities offline`. Both lines appear in the unpatched
  baseline and in both probe runs, so they are pre-existing and not caused by
  the probe. [GUEST baseline and both probe runs]
- **Gap:** a C filesystem API would be a broker library over filesd
  endpoints (report 04, option b). Nothing in the C runtime touches files.

## 9. Process lifecycle

- Creation: `SYS_SPAWN` (12) takes an exact Image capability, an explicit
  inheritance spec (at most 8 inherited caps) and an optional notification
  badge. It returns a Process handle. The child does **not** clone the
  parent's cap table. Limits: 64 live processes, 64 spawn records, and
  dynamic children at most 24. [CODE]
- Boot-time creation: `spawn_init` and `spawn_init_boot` are kernel-literal
  paths (`entry.rs`). Their grants are fixed in code.
- Exit: `SYS_THREAD_EXIT` (2) records the status, wakes any joiner, and
  records process status when the last thread leaves. `SYS_PROC_STATUS`
  (61) and `SYS_WAIT` (11) observe it. **The serial log does not print exit
  codes**, so the probe's exit value (37 on success) is not independently
  observed. [CODE sys_thread_exit at syscall.rs:876; GUEST]
- Teardown is documented as frame-exact (`docs/ARCHITECTURE.md`). The M-suite
  PASS lines in serial are the evidence that the boot-time teardown paths run.
  [CODE, GUEST suite PASS lines]

## 10. Time

- `SYS_CLOCK_NOW` (26) returns monotonic microseconds with **no capability
  required**. `SYS_RTC_READ` (50) reads wall time. `SYS_TIMER_ARM/CANCEL`
  (27/28) deliver a badge on a notification, limited to 4 timers per process.
  [CODE]
- Covert-channel note: an unprivileged monotonic clock is ordinary for this
  class of system. Noted for completeness.

---

## Summary of audit findings

| ID | Area | Finding | Severity for the C path | Evidence |
|---|---|---|---|---|
| F1 | syscall | `SYS_DEBUG_WRITE` has no capability check; any process can write serial | High (must not ship to C stdio) | CODE |
| F2 | stdio | No C binding to native stream rings | High (blocks any real C program output) | CODE |
| F3 | TLS | ELF loader ignores PT_TLS; runtime must do TLS | Medium (works; loader change needs ADR) | GUEST |
| F4 | memory | No coalescing in the C allocator; measured refusal under churn | Medium (process-local DoS) | HOST |
| F5 | memory | Rust ScalableHeap not exposed to C | Medium | CODE |
| F6 | threads | No C thread or mutex wrappers | Medium (blocks SDL3 / engines) | CODE |
| F7 | startup | No argc/argv/env contract for raw C images | Medium | CODE |
| F8 | exit | Exit codes not logged; harness cannot prove exit value | Low | CODE, GUEST |
| F9 | ABI check | Cross-check accepts constants absent from the kernel | Low | STATIC |
| F10 | loader | Dynamic images capped at 256 KiB / 128 pages | Low for prototype, High for big apps | CODE |

Core-OS dependencies for any real C workload are listed in report 05.
