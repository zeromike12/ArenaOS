# ADR-0108: Defer native PIE and ASLR

## Status

Accepted for Phase 13; revisit in the next native executable phase.

## Context

The qualified and Phase-13 application launch paths both resolve an exact
Image capability and use the kernel's existing ELF loader. The loader accepts
the ArenaOS strict subset of static ELF64 `ET_EXEC`: it maps page-aligned
`PT_LOAD` segments at their declared virtual addresses, rejects writable and
executable segments, and rejects `PT_INTERP`. `ET_DYN` is refused explicitly.
The kernel Image registry uses the same validator; it does not introduce a
second executable format or relocation path. The historical M4 rejection
fixture also checks that changing the header to `ET_DYN` is refused.

Phase-13 native binaries are linked with `relocation-model=static` and
`-no-pie`. Startup ABI v2 reports the validated fixed image base. There is no
native `PT_DYNAMIC` parser, relocation table validation/application, or
per-process randomized image placement. No dynamic linker is present.

## Decision

Keep Phase-13 installed applications on the existing static `ET_EXEC`
contract. Do not accept `ET_DYN` or claim ASLR support in this phase.

Adding even a static PIE subset requires a coordinated loader change: compute
and validate a load bias, validate and apply only the chosen relocation
records (initially `R_X86_64_RELATIVE`), reserve randomized image ranges with
guard gaps, and carry the actual base through Image metadata and Startup ABI
v2. The work also needs an ArenaOS-native source and policy for per-launch
placement, plus positive and malformed relocation fixtures through the real
installed-package and process-spawn path. None of these operations can be
expressed by the current fixed-address `ET_EXEC` validator.

This phase already changes process-wide memory ownership, protection,
teardown, user-thread context switching, synchronization, package discovery,
and multi-window lifecycle. Extending those high-risk paths with a new ELF
loader and address-randomization contract would widen the preservation surface
without being required for the installed native application workload. The
existing loader's strict W^X and user/kernel bounds remain in force.

## Consequences

- Phase-13 applications remain static non-PIE `ET_EXEC` images at the
  existing validated addresses; no ASLR guarantee is made.
- Installed package verification and exact Image authority are unchanged.
- The next native executable specialist scope should implement and qualify
  static `ET_DYN` with a narrowly accepted relative-relocation subset,
  randomized load bias, guard gaps, and W^X before dynamic linking or Linux
  executable compatibility is considered.

## Evidence

- `kernel/kernel/src/elf.rs` checks `ET_EXEC`, rejects `PT_INTERP`, and maps
  only validated `PT_LOAD` segments.
- `kernel/kernel/src/m4.rs` includes an explicit `ET_DYN` rejection case.
- The native `x86_64-unknown-none` app target config uses
  `relocation-model=static` and `-no-pie`.
- Installed startup audit fixtures require the current fixed `load_base`.
