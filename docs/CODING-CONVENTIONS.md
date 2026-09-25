# ArenaOS — Coding Conventions

## Rust

- Stable toolchain only, edition 2024. No nightly features in OS code.
- `rustfmt` defaults (`rustfmt.toml` at repo root); format before commit.
- `clippy` clean for the target (`cargo clippy`); warnings are errors in the
  kernel workspace (`#![deny(warnings)]` in release-critical crates once the
  tree stabilizes; currently lints are treated as must-fix in review).
- `unsafe` rules:
  - Every `unsafe` block/function carries a `// SAFETY:` comment stating the
    invariants that make it sound.
  - `unsafe_op_in_unsafe_fn` is denied: unsafe fns still need explicit
    `unsafe` blocks inside.
  - No `static mut`. Shared mutable state uses `UnsafeCell` wrappers,
    atomics, or (later) proper synchronization — with a comment explaining
    why it is safe at boot time if pre-SMP assumptions are used.
  - Prefer quarantining `unsafe` into tiny leaf modules
    (`arch/`, `drivers/serial.rs`) over sprinkling it through logic.
- Panics: the kernel must not panic in production paths. `panic!`/`unwrap()`
  are for boot-time self-tests and "impossible" states with a comment; the
  panic handler prints diagnostics and halts (later: reboots/parks
  gracefully). Library-style `Result` for anything fallible.
- Floating point: avoided in kernel code (soft-float target); when truly
  needed (calibration math), integer-only formulations preferred.
- No `std`, obviously; `alloc` usage in the kernel starts at M2.5 with the
  kernel heap and goes through our allocator types, not a global `#[global_allocator]`
  (until an ADR says otherwise).

## Naming & structure

- Modules ≤ ~400 lines; split by responsibility, not by size alone.
- Names describe roles: `frame_allocator`, `boot_info`, `endpoint` — no
  `util`, `misc`, `common`, `helpers`.
- One subsystem per directory; cross-subsystem imports go through the
  subsystem's `mod.rs` public surface.
- Arch-specific code lives **only** under `arch/<name>/` behind arch-neutral
  interfaces (RISK R10). Anything that says `cr3`, `lgdt`, `syscall` in its
  body belongs there.

## Constants & hardware facts

- No magic numbers. Named constants with unit suffixes where relevant
  (`BAUD_DIVISOR_38400`, `COM1_BASE_PORT`).
- Spec references in comments for every hardware/ABI fact:
  `// UEFI 2.10 §8.2, EFI_MEMORY_DESCRIPTOR`, `// SDM Vol. 3 §6.11, IDT`.
- `#[repr(C)]` for every ABI struct + static size/offset assertions:
  `const _: () = assert!(core::mem::size_of::<MemoryDescriptor>() == 48);`

## Logging

- Kernel logs go through `log.rs` (level + module tag), never raw prints:
  `[arena INFO boot] ...`, `[arena WARN serial] ...`.
- Test markers use the exact grammar from TESTING.md.
- Log lines are ASCII, `\r\n` terminated (serial), and greppable
  (`key=value` for structured fields).

## Commits & repo hygiene

- The tree builds and passes `tools/run_tests.sh` at every commit.
- Commit messages: imperative subject ≤ 72 chars; body explains *why*;
  reference milestone (`m1:`, `m2.3:`) and ADRs where relevant.
- No generated artifacts in git: `build/`, `kernel/target/`, images, logs are
  gitignored.
- Docs are part of the change: a milestone without updated ROADMAP status and
  (if applicable) an ADR is incomplete.

## Review checklist (self-review until the team grows)

1. Does it compile warning-free for `x86_64-unknown-uefi`?
2. Does every `unsafe` have SAFETY? Every ABI struct an assertion?
3. Is there a test that would fail if this code were removed or faked?
4. Did any arch-specific knowledge leak outside `arch/`?
5. Did scope creep past the current milestone (ROADMAP firewall)?
6. Are docs/ADR/ROADMAP updated?
