# C1 plan: native C application platform

Branch: `arena/f89ea987-arenaos`. Owner role: C runtime and application
compatibility. Status: **experimental.** The plan records what was attempted,
in what order, and the verification rules. Results live in
`C1-FINAL-REPORT.md`.

## 1. Goal and acceptance

Goal: an ordinary signed C application launches inside ArenaOS through the real
capability lifecycle, uses granted native streams, and exits with an observable
status. Native threading and synchronization are the next major deliverable and
must be independently guest-qualified.

Constraints carried into every step:

- No production kernel change. No syscall ABI or executable loader change without
  an ADR. No new ambient authority. No weakened W^X or capability check.
- No dynamic linker, no Linux ELF compatibility, no POSIX filesystem namespace,
  no hosted libc, no duplication of Luna's PIE or ELF work.
- No SDL3 port or build, no graphics, no change to Arena 1's branch, no change to
  Luna's branch (`arena/phase14-native-pie-aslr`).
- Keep the scratch boot probe and the crt-probe as legacy regressions. They do
  not substitute for C1.2.
- The RFC 8032 development fixture is for tests only, never a production key.
- No PR, merge, release, or session end as part of this assignment.

## 2. Sub-milestones

| Step | Scope | Planned evidence | Status at end of C1 |
|---|---|---|---|
| C1.1 | Granted stdin, stdout, stderr; bounded I/O; partial transfer; EOF; peer closure; backpressure; no `SYS_DEBUG_WRITE`; clean missing-grant failure | HOST-ONLY ring and stdio tests; GUEST native app plus stream-less negative package | Implemented. Guest-tested. Partial and error paths host-only. Ring wrap blocked (ADR-0111). |
| C1.2 | Signed native C app through APB1 and the normal lifecycle; entry, ABI validation, TLS, runtime init, stdout, allocate, time, logic, exit status | GUEST: harness installs, launches, judges with exact verdicts; repeated runs | Implemented. GUEST 5/5 clang, 5/5 gcc on the final tree. |
| C1.3 | Threads: create, join, detach, exit, yield; mutex; condvar; once; TLS isolation; cleanup; quota; stale handles | GUEST T8 and T9 inside the native app | Implemented and guest-tested in T8 and T9. **Independent C1.3 harness not built.** |
| C1.4 | Allocator hardening; ownership checks; realloc; invalid, interior, and freed pointers; overflow; alignment; bounded coalescing; VM failure without corruption; thread safety | HOST-ONLY (ASan+UBSan); GUEST T5, T9, and crt-probe | Implemented. Host covers VM refusal. **Guest VM commit-failure test not done.** |
| C1.5 | SDL3 handoff: calling convention, linking, startup, allocation, libc subset, streams, threads, TLS, timing, FP, limits, missing symbols, capabilities, blockers | Document, checked against source and guest evidence | Written: `C1-SDL3-HANDOFF.md`. |
| FP/SIMD | Audit of CPU FP and SIMD state; core-OS dependency for Luna after Phase 14; no kernel FP code | STATIC audit of the kernel and every C image | Written: `C1-FPSIMD-BOUNDARY.md`. Decision left to Luna. |
| Docs | `C1-PLAN.md`, `C1-INTEGRATION-CONTRACT.md`, `C1-FINAL-REPORT.md`, `C1-SDL3-HANDOFF.md`, ADR-0110 addendum, ADR-0111 (new, proposed) | Each claim labelled | Written. Reviewed against source and logs in this pass. |

## 3. Order of work actually followed

1. Prototype runtime and allocator (`da0bfa4`), HOST-ONLY.
2. Native C app and signed APB1 harness (`c244239`), with exact verdicts, and the
   two-package negative test.
3. Crt-probe parameter change to the new heap contract (`b4e790c`), keeping the
   legacy probe as a regression.
4. C1.4 concurrent allocator check inside the native app (`871f178`) and host VM
   refusal injection (`f38ad6a`).
5. C1.1 missing-grant fix (`9238c97`), then the printf short-write and error fix
   (`2e8fae6`), HOST-ONLY.
6. Audit rule correction (`76a55ee`), then the stream-less package, stderr-first
   probe, and console-safe judge (`bcdfb95`), with guest evidence.
7. Repeated guest runs on the final tree and the documents.

## 4. Verification rules (applied, not aspirational)

- The harness fails on any missing required group line, any failed check, any
  unexpected exit status, and any malformed verdict. A bare `CRT-PROBE RESULT`
  line is not success; the exact `CRT-PROBE RESULT PASS (6/6)` is required.
- Exit statuses are asserted: 57 for the native app, 58 for the stream-less
  negative package, with exit order `[58, 57]`.
- Artifact identity is recorded in `build/guest-native/bundle-identity.json` and
  `summary-<cc>.json`: source commit, source file hashes, ELF and bundle hashes,
  and the boot image hash.
- Evidence labels are kept apart: HOST-ONLY, STATIC, and GUEST.
- Host success is not reported as kernel memory safety.
- Stale evidence is not counted. The summaries record the dirty-tree state, and
  the final report lists only evidence on the final tree.

## 5. Issues found during C1 and how they were handled

| Issue | Handling |
|---|---|
| A run failed on a missing T2 line while the RESULT line said 9/9. Suspected dropped stdout bytes. | Checked the full log. The bytes were present, split by a kernel log line in the desktop's chunked relay. `arena_stream_write` already loops. The judge now matches app lines on a view without whole log fragments. Exact checks remain elsewhere. |
| `sink_flush` ignored the return of `arena_write`, so a short write or peer closure could be reported as success. | Fixed in `stdio.c`. Host tests cover 3-byte short writes, output above 128 bytes, a single failed attempt, and no retry after failure. A mutation control confirmed the tests fail on the old behaviour. |
| The static audit flagged any mnemonic starting with `f`, which matched string data in the executable segment. | Replaced with an explicit x87 and FP-state mnemonic list. The register-operand rule stays. |
| The desktop exit receipts came from a worker that outlived `main`. | The app waits, with a bound, until `arena_thread_count()==1` before returning. |
| The crt-probe failed 5/6 on the new heap layout. | Changed the probe's parameters to the new contract. The allocator was not reverted. |
| The kernel's CR4 handling was described in a draft as setting PAE, SMEP, and SMAP. | Corrected from source: the kernel read-modify-writes the firmware value and only ORs in SMEP and SMAP. |
| A draft claimed a target-feature result from `rustc`. | `rustc` is not on the sandbox PATH now. The claim is marked unverified and is not relied on. |

## 6. Decisions taken autonomously

- Kept the stream and APB1 protocols unchanged. The ring wrap defect is
  documented and not fixed, because the fix changes the wire format.
- Kept `SYS_DEBUG_WRITE` out of the C path. Did not add any syscall.
- Used the existing `arena_` public names and six standard aliases. No libc
  shim was added.
- Chose a spinlock with yield backoff for the allocator. A contended acquire
  calls `arena_backoff()`, which yields. The lock is held across `arena_vm_commit`
  and the lazy `arena_vm_reserve` (`alloc.c`, lines ~98 and ~158). Those VM calls
  are assumed not to block. That assumption is recorded here and is a review
  point. If a VM call can park, the lock must be redesigned. Evidence: host
  stress, guest T5 and T9.
- Judged serial output on an app-line view, not raw lines. Exit status and the
  RESULT line stay exact.

## 7. Escalations (architecture)

Escalated to the owners in the reports, with no change landed:

1. Stream ring counter wrap (ADR-0111, proposed): M and C review. Blocks streams
   for unbounded output.
2. `SYS_DEBUG_WRITE` capability check (ADR-0110 Decision 3): core OS.
3. FP/SIMD for C (`C1-FPSIMD-BOUNDARY.md`): Luna after Phase 14.
4. PT_TLS ownership (ADR-0110 open question): Luna loader owners.
5. Dynamic loading and optional backends for SDL3 (`C1-SDL3-HANDOFF.md` B2): Arena 1 and C.

## 8. Remaining gaps (not done in C1)

- Guest-side VM commit failure (C1.4). A test needs a fault that the userspace
  process can reach without a kernel change.
- An independent C1.3 harness. T9 runs inside the C1.2 app.
- A recorded hash for the clang stream-less package. The harness keeps only the
  last-built compiler's negative package in `bundle-identity.json`.
- Re-verification of the kernel target's default features (`rustc` not on
  PATH).
- A soak test for threads and streams.

## 9. Reproduction

See `C1-FINAL-REPORT.md` §6 and `C1-SDL3-HANDOFF.md` §14. All guest commands
need QEMU and the normal Phase-13 disk images. Generated artifacts stay under
`experiments/c-runtime/build/` and are not committed.
