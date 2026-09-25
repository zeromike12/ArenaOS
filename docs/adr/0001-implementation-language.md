# ADR-0001: Implementation language — Rust (stable, `no_std`)

Status: Accepted
Date: 2026-09-25 (project start; ratified at Milestone 1)
Milestone context: Phase 0 — Architecture

## Problem

The kernel, boot stage, drivers, and userspace all need an implementation
language. This choice propagates into every file we ever write and is
extraordinarily expensive to reverse, so it must be made before significant
implementation begins.

## Options considered

**C.** The proven default (Linux, FreeBSD, seL4's heritage). Excellent
toolchain and zero surprises. Rejected as primary language: memory-safety
classes of bugs (use-after-free, buffer overrun, data races) are precisely the
bugs that dominate kernel CVE history, and this project's security posture
(§ARCHITECTURE 9) would be undermined at the source level. A 20-year codebase
of C maintained by a small team is a 20-year stream of such bugs.

**C++.** RAII and types without giving up native control. Rejected: ABI
fragility across toolchains, heavyweight build ecosystem, and the safety story
is opt-in discipline rather than enforced by the compiler; undefined behavior
remains pervasive and silent.

**Zig.** Modern C replacement with a superb cross-compilation toolchain and
explicit comptime. Viable candidate; rejected as primary: younger ecosystem,
no borrow checker (aliasing/mutability bugs still possible), unstable language
spec during the exact years this project would be defining its foundations.

**Rust (chosen).** Memory safety without GC, data-race prevention at compile
time, algebraic types that make state machines (VM states, IPC states) hard to
misuse, `no_std` designed for exactly this job, first-class LLVM toolchain,
and a real track record in kernels (Linux/Windows/Android drivers, seL4
verification-adjacent work, Redox, Theseus) and firmware.

## Decision

- **All new code is Rust, stable toolchain only** (no nightly features;
  `RUSTC_BOOTSTRAP` is used *only* by our offline sysroot bootstrap to build
  `core`/`alloc` for bare-metal targets — a build-tool concern, never a
  kernel-code concern).
- Inline assembly (`core::arch::asm!`) is permitted only where hardware
  demands it: boot transitions, context switching, control-register access,
  syscall entry. Each such block requires a `// SAFETY:` comment.
- C is not used, not even for drivers, until a concrete need appears (e.g.,
  porting an unreplaceable reference implementation — none anticipated).
- `unsafe` is allowed in the kernel but quarantined: small modules, mandatory
  SAFETY comments, `unsafe_op_in_unsafe_fn` denied.

## Reasoning

Safety is the highest-leverage property for a small-team, long-horizon kernel:
it eliminates entire bug classes that are otherwise found in production, in
the worst possible place, years later. Rust's cost — toolchain bootstrap
complexity, borrow-checker friction in genuinely-shared-state code — is real
but front-loaded and documented; C's cost is permanent and back-loaded.

## Downsides accepted

- Bare-metal Rust requires bootstrapping `core`/`alloc` for our targets in an
  offline environment (we maintain `tools/build-sysroot.sh` for this; it is
  a documented, pinned, reproducible step).
- No stable `async` in kernel-relevant contexts; we do not want it there
  anyway (IPC is synchronous rendezvous + notifications by design).
- Rust's `core` is large; we accept its size and pull only what we use
  (linker GC of unused objects keeps images small — M1 image is ~6 KiB).

## Future implications

- Toolchain version is pinned and upgraded deliberately (recorded in
  `docs/DEV-ENV.md`), not floating.
- If a future need arises for another language (e.g., assembly-heavy crypto,
  or a certified component), it requires a superseding ADR.
- Userspace APIs will be Rust-native; bindings for other languages are a
  Phase 8+ ecosystem question, deliberately deferred.
