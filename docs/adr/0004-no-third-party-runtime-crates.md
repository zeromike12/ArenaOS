# ADR-0004: Zero third-party runtime crates; own UEFI bindings

Status: Accepted
Date: 2026-09-25
Milestone context: Phase 0 — Architecture, exercised in Milestone 1

## Problem

The Rust ecosystem offers ready-made building blocks for OS work (`uefi`,
`x86_64`, `linked_list_allocator`, `spin`, ...). Decide now, before
dependencies accumulate, what external code may live inside the OS image.

## Options considered

**Use the standard osdev crates** (`uefi`, `x86_64`, etc.): fastest start,
well-tested by the community. Rejected as *runtime* dependencies: they are
large surfaces we do not control, their releases move independently of our
needs, and "do not casually import large existing components" is a project
rule. Our UEFI needs in the boot stage are a few tables and a memory-map
call — a fraction of what `uefi` provides.

**Vendor crates into the repo:** better control, but inherits their design
decisions and update burden; vendoring is a large surface to audit.

**Write our own minimal bindings (chosen):** hand-write the UEFI ABI structs
we actually use, with `#[repr(C)]`, `extern "efiapi"`, and compile-time size/
offset assertions. Same policy for future needs: our own register accessors,
our own allocators.

## Decision

1. **The OS image (boot stage, kernel, servers, drivers) contains zero
   third-party crates.** Only `core`/`alloc`/`compiler_builtins` from our own
   bootstrapped sysroot, and our own code.
2. **Development tooling may use external components** (pip/npm/git packages
   for QEMU, firmware blobs, image tools, the Rust toolchain itself) — these
   never execute inside ArenaOS; they are pinned and documented in
   `docs/DEV-ENV.md`.
3. Hand-written hardware/spec bindings must carry: spec reference comments
   (e.g., "UEFI 2.10 §4.2 Table 10"), `#[repr(C)]`, and `const _: () =
   assert!(size_of::<T>() == N)` style checks where the spec fixes sizes.
4. This policy is also forced-friendlier to our build environment: the
   development sandbox has no crates.io access, so a zero-dependency kernel
   builds *identically* here and anywhere else. What is a constraint today is
   a supply-chain-security property forever: our trusted computing base
   contains no code we did not read.

## Reasoning

The trusted computing base of an OS is its kernel, its boot chain, and every
crate statically linked into them. A capability-based security architecture
(ADR-0002) built on unaudited third-party ring-0 code would be self-defeating.
The volume of code we actually need from the ecosystem is small and
mechanical; writing it ourselves is measured in hundreds of lines, once, and
becomes documentation-grade knowledge of the specs we depend on.

## Downsides accepted

- Slower start for each new hardware interface (we write the bindings before
  we use them).
- We own the bugs in those bindings — mitigated by spec-cited comments,
  static assertions, and boot tests against real EDK2 firmware.
- Reinventing solved problems (e.g., allocators) later; when we do, we study
  the prior art and then write our own, deliberately (Phase 2 ADRs will cover
  allocator design).

## Future implications

- If someday a crate proves genuinely irreplaceable (e.g., a formally verified
  crypto primitive), adopting it requires a superseding ADR plus an audit
  plan — not a casual `Cargo.toml` edit.
- Userspace libraries we write (async runtime, toolkit) follow the same
  policy: they are *our* ecosystem's packages, not imports.
