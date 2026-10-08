# C1 integration contract: what ArenaOS already provides to a native C application

Status: **experimental, host and guest evidence as labelled in
[C1-FINAL-REPORT.md](C1-FINAL-REPORT.md)**. Nothing here is a production
interface. Every item is one of:

- **EXISTING**: present in the shipped kernel / Phase-13 runtime and used
  unchanged by the C1 prototype.
- **EXPERIMENTAL-BINDING**: a C-side binding of an EXISTING contract, under
  `experiments/c-runtime/`. It adds no kernel code and no syscall.
- **MISSING**: needed for the target, not present; each row names the owner
  and the blocker.

Sources read: `kernel/kernel/src/arch/x86_64/syscall.rs`,
`kernel/kernel/src/sched/mod.rs`, `kernel/kernel/src/sync_domain.rs`,
`userspace/abi.rs`, `arena-startup-abi`, `arena-platform` startup,
`arena-runtime` (startup, streams), `desktop.rs` (headless launch and
stream grants), `tools/apb1_format.py`, `tools/mtest.py`,
`tools/test_phase13_registry_guest.py`, `tools/test_m10_apps.py`,
`tools/startup_abi.py` (read in this pass; see M9), `userspace/phase13-headless/src/main.rs`
(Rust reference fixture for the same ARST, SyncDomain, thread, stream, and
timer contracts; read in this pass), and the `arena-runtime` `threads.rs`,
`sync.rs`, `tls.rs`, `heap.rs`, `vm.rs`, and `capabilities.rs` module headers
(read in this pass; the bodies were consulted by targeted search).
Constants in `experiments/c-runtime/include/arena/abi.h` are cross-checked
against `userspace/abi.rs` by `run.py abi` (90 constants, rc 0).

## 1. Process entry and startup (EXISTING, consumed by EXPERIMENTAL-BINDING)

| Item | Contract | C1 binding |
|---|---|---|
| Image format | APB1 signed bundle; manifest kind 1 = executable | `tools/apb1_format.py` packages `bin/cnative` (flags 28) |
| Entry | ELF `ET_EXEC`, static, non-PIE, loaded by the existing loader | `crt0.c` `_start`; `__arena_tls_*` symbols from `link/arena-user.ld` |
| Startup ABI v2 | ARST block, 4096 B, page in slot 0, `SharedRegion READ\|DESTROY`; header 128 B | `startup.c` validates before `main` |
| Startup gate | slot 0 must be SharedRegion READ\|DESTROY; page mapped, copied, unmapped, slot destroyed; offsets, role uniqueness, strings, reserved zeros, and live table checked | identical checks in C; failure gives `ARENA_E_STARTUP` (refusal before entry; the Phase-13 suites own the red controls, not re-run in C1) |
| Manifest flags | MULTI_INSTANCE 1, BACKGROUND 2, HEADLESS 4, STANDARD_STREAMS 8, NATIVE_SYNC 16 (mask 0x1F) | C1 uses 28 = HEADLESS\|STANDARD_STREAMS\|NATIVE_SYNC |
| argv | `argv[0] = application_id` on the headless path | `arena_startup()->argv` |
| Stack | runtime-owned, bounds from `arena_stack_bounds` | `crt0` switches stack, then calls `main` |
| TLS | `fs_base` set by `ARENA_SYS_TLS_SET`; runtime installs the block | `crt0.c`; loader does **not** process `PT_TLS` (F3) |

Limits that matter to C (EXISTING): dynamic image cap 256 KiB / 128 pages
(F10); MAX_SEGMENTS 8; CAP_SLOTS 128 per process; MAX_INHERIT 8.

## 2. Standard streams (EXISTING contract, EXPERIMENTAL-BINDING)

- **Discovery.** Roles in ARST: Stdin 2, Stdout 3, Stderr 4, StreamSet 6,
  StreamWake 7. `stdin`, `stdout`, `stderr` all refer to the single StreamSet
  index. A stream grant that is missing fails cleanly (`arena_stdio_init`
  returns an error; the app records `stream roles` and `stdio attach` checks).
- **Capabilities.** StreamSet is a SharedRegion READ\|WRITE; StreamWake is a
  Notification with WRITE only. The runtime cross-checks both with
  `ARENA_SYS_CAP_DESCRIBE` before mapping, then `ARENA_SYS_SHARED_PAGES` must
  return 1 and `ARENA_SYS_SHARED_MAP(slot,1)` maps the page. Any failure
  unmaps.
- **Ring protocol** (ADR-0104, unchanged): 64-byte header `"ASTR"`, version 1,
  3 channels, capacity 768; ring *i* at `64 + 832·i` with write counter @0,
  read counter @4, state @8, data @+64. State bits: WRITER_CLOSED 1,
  READER_CLOSED 2.
- **Behaviour.** Writes are short when the ring is partly full; `WouldBlock`
  when full; `BrokenPipe` if the reader closed; `Closed` if the writer
  closed. Reads return 0 only after the writer closed and the ring is empty.
  Wake is `ARENA_SYS_NOTIFY` on the WRITE-only cap; waiting is
  `ARENA_SYS_WAIT` on the Notification. **No new syscall is used.**
- **Known protocol defect (blocker, see §8 and ADR-0111 proposal).** Both
  counters are u32 and are indexed with `counter % 768`. Because
  2^32 mod 768 = 256, the mapping is discontinuous at the 4 GiB wrap. The
  host suite pins this as a KNOWN-DEFECT; the fix changes the wire format and
  needs an ADR. The C1 prototype does not change the wire format.
- **FD numbers** are local to the C process and grant no authority. The C
  binding maps fd 0/1/2 to the granted roles only.
- **No `SYS_DEBUG_WRITE` on any supported stdio path.** `arena_write` routes
  to `arena_stream_write` unless `arena_legacy_serial_enable()` was called;
  only the legacy boot probe calls it. The C1 application never does.

## 3. Threads and synchronization (EXISTING syscalls, EXPERIMENTAL-BINDING)

| Call | Kernel behaviour (EXISTING) | C binding |
|---|---|---|
| `THREAD_CREATE` | stack cap VmRegion READ\|WRITE\|DESTROY (no COPY); stack_low/stack_top inside region; fs_base 16-aligned in page 0 with 16 writable bytes; initial RSP = stack_top−8; entry is a present, non-writable user page; QUOTA on refusal, BAD_ADDRESS on bad address, BAD_ARG for bad ids | `arena_thread_create` |
| `THREAD_JOIN` | same-process only; writes an 8-byte status; second join refused (`ARENA_E_STALE`, tested in T9) | `arena_thread_join` (returns `ARENA_E_STALE` after detach/second join) |
| `THREAD_DETACH` | detached zombie reaped by scheduler | `arena_thread_detach` |
| `THREAD_EXIT` / `YIELD` / `COUNT` | COUNT = live non-zombie threads of the process | `arena_thread_count`, `arena_yield`, `arena_busy_sleep_us` |
| Limit | MAX_USER_THREADS_PER_PROCESS 4 (initial thread excluded); MAX_THREADS 64 system-wide | quota refusal tested in T9 (fifth user thread gives `ARENA_STATUS_QUOTA`) |
| Process status | recorded from the **last** thread to exit (`record_process_status_if_last`) | C main must wait for detached workers before returning (fixed in C1.2 app; see §7) |
| Sync domain | MAX_DOMAINS 64; 32 keys/domain; MAX_KEYS 2048; mint via `SYNC_DOMAIN_CREATE` (NATIVE_SYNC flag) | `arena_sync_init` |
| Keys | create/destroy/sequence/wait/wake/info; stale or wrong-generation token gives BAD_ARG; key exhaustion gives QUOTA; destroy with parked waiter gives BUSY; wait with IF set gives BUSY; timeout over 86 400 000 000 µs gives BAD_ARG | `arena_mutex_*`, `arena_condvar_*`, `arena_once_*` |
| Mutex fast path | local, kernel consulted only when parking | as designed |

Not used anywhere: Linux futex. Not implemented: thread-local destructors,
cancellation, signals, `pthread`-style attributes.

## 4. Memory (EXISTING VM syscalls, EXPERIMENTAL-BINDING)

- VM: 8 regions per process; 4096 pages per region; 8192 committed pages per
  process; 64 pages per operation (`VM_RESERVE`, `VM_COMMIT`, `VM_RELEASE`).
- Rust reference heap (`arena-runtime/src/heap.rs`, read in this pass) is a
  different design: single-user-thread, frame-mapped through `SYS_ALLOC_FRAME`
  and `SYS_MAP_MEMORY`, 4 080-byte maximum block, a 32-page ceiling, and mapped
  pages retained until process exit. The C allocator does not use it.
- C heap (`alloc.c`): one 16 MiB reservation (4096 pages), committed in
  64 KiB chunks, power-of-two blocks 32 B..16 MiB, 16-byte headers, LIFO
  free lists, bounded coalescing, ownership validation before metadata reads,
  overflow-checked size arithmetic, spinlock for thread safety.
- Refused commit or reserve leaves free lists and headers unchanged. Host
  coverage: `test_vm_commit_refusal` and `test_vm_reserve_refusal` (commit
  `f38ad6a`). **Guest coverage of a refused VM commit is NOT present in C1**;
  the crt-probe refusal counters exercise the heap ceiling, not a kernel commit
  failure. This is an open C1.4 gap (see the final report).
- `ScalableHeap` (kernel-side allocator used by Rust userspace) is **not**
  exposed to C (F5). The C heap is a separate implementation.

## 5. Time (EXISTING, EXPERIMENTAL-BINDING)

`ARENA_SYS_CLOCK_NOW` gives monotonic microseconds. `arena_clock_us` reads it.
The C1 group T6 checks monotonic non-decrease and that a 20 ms sleep covers
at least 20 ms. Observed over ten logged native runs (5 clang, 5 gcc): the
sleep measured 121 185–181 398 µs. The timer is coarse on this VM; C1 claims
the lower bound only, not timing precision.

## 6. Capabilities and authority (EXISTING, unchanged)

- Grants are capability slots; a C process only sees slots in its ARST table.
- Copy attenuates only; move clears the source; destroy is right-gated;
  dangling caps fail invoke (M3 guest results).
- Paths, PIDs, and names confer no authority. The C runtime does not add any
  ambient authority, privileged bypass, or image-ID special case.
- Launch is by the normal desktop/AFS2 path: install through the protected
  APB1 install capability, then launch from All Applications. The install
  capability is refused to ordinary Filesd callers (the message is asserted by
  `tools/test_phase12_apb1_guest.py`; it was not re-run in C1).

## 7. Lifecycle and exit observation (EXISTING)

- Exit status: `ARENA_SYS_THREAD_EXIT(status)` on the last thread; the
  desktop observes it through the Process capability and logs
  `[desktop] child Process-cap exit status=<n>` (guest-observed as 57 in C1).
- The kernel does **not** print exit codes (F8); the desktop does.
- Lesson recorded from C1: a detached worker that is still alive when `main`
  returns becomes the last thread and sets the observed status. The C1 app now
  waits (bounded, checked) until `arena_thread_count() == 1` before returning.

## 8. Missing interfaces and blockers

| # | Missing / broken | Owner | Impact on C1 / SDL3 | Status |
|---|---|---|---|---|
| M1 | `SYS_DEBUG_WRITE` has no capability check (F1) | Core OS + architecture review | Excluded from C path; kernel still has it | Proposed in ADR-0110 Decision 3; not landed |
| M2 | Loader ignores `PT_TLS` (F3) | Luna loader owners | C runtime installs TLS itself; `link/arena-user.ld` KEEP contract is fragile | Open option in ADR-0110 |
| M3 | Ring counter wrap at 2^32 bytes (streams.rs, streams.c) | Phase-13 stream owners | Corruption after 4 GiB per channel | Pinned defect; ADR-0111 proposed (wire-format change) |
| M4 | No FP/SIMD state save/restore; kernel never sets CR4.OSFXSR/OSXSAVE | Core OS (Luna review after Phase 14) | User code must not use FP/SIMD; C audit enforces it | See C1-FPSIMD-BOUNDARY.md |
| M5 | No PIE / relocation | Luna Phase-14 | C images are static at 0x200000 | Out of C1 scope |
| M6 | No dynamic loading, no POSIX FS namespace, no hosted libc | Architecture | SDL3 needs a static, no-optional-backend build | Documented in handoff |
| M7 | No blocking stdin wait API in C (only bounded nonblocking reads plus notification wait) | C runtime (experimental) | Event loops must poll | Documented; future binding |
| M8 | Kernel-side memory safety of the C heap | Not provable from host ASan/UBSan | Host results are HOST-ONLY | Stated in final report |
| M9 | `tools/startup_abi.py` sets `CAP_MAX = 4`, which its validator enforces (`capc > CAP_MAX` is refused). The authority, `arena-platform` `CAPABILITY_MAX`, and the C header are both 7. `run.py abi` does not check this Python file, so the mismatch is not caught by any gate. | Phase-13 tooling owner (M) | Records with 5 to 7 capabilities would be refused by the Python validator. The C runtime and the Rust platform accept 7. | Found in C1; not changed (production tool). Recommend M decide whether 4 is intentional, and if not, align to 7 and add a test. |

Nothing in this table was resolved by guest-unverified code.

## 9. Consistency checks made in this pass

- ARST offsets used by the C parser (`startup.c`: instance 48, generation 52,
  argc 60, envc 62, capc 64, args 68, env 72, caps 76, strings 80, strings_len
  84, page size 96, entry 104, base 112) match `tools/startup_abi.py`. The
  Python `OFF_CLOCK` at 120 is not read by C.
- ARST argument and environment maxima (32 and 32) match in C, Python, and Rust.
  The capability maximum does not match in Python (M9).
- TCB layout: the self-pointer is at `%fs:0` in C and at offset 0 in the Rust
  `ThreadControlBlock` (`tls.rs`). Both use FS.base as the TCB contract.
- Thread regions (exact VM reservation, guard page, committed RW/NX stack) and
  sync keys (generation-checked, in the granted SyncDomain) match the Rust
  design in `threads.rs` and `sync.rs`. Items M1, M3,
M4 require architecture review before any change.
