# ADR-0093 — Bounded native Image envelope for installed applications

**Status:** Accepted for Phase-13 implementation; guest qualification pending.
**Date:** 2026-10-07.
**Related decisions:** ADR-0016, ADR-0055, ADR-0083, ADR-0086, ADR-0092.

## Context

The Phase-12 dynamic Image registry has two fixed 4 KiB entries and permits
four live dynamic-image child processes. It validates static ET_EXEC images,
at most 16 mapped load pages, and returns an exact Image capability. Those
bounds were suitable for APKG proof fixtures, but are below APB1's installed
executable size and the Phase-13 mixed application workload.

Desktop also needs the loader-validated ELF entry point and load base to encode
the existing Startup ABI v2 block for a dynamically registered Image. It
must not trust fields supplied separately by an application catalog or
reparse an Image differently from the kernel.

## Decision

- Keep the current possession-gated `SYS_IMAGE_REGISTER` authority and
  one-time kernel-owned byte copy. A caller must hold the live ImageRegistrar;
  every input byte is copied only after its full user range is validated.
- Increase the dynamic registry to 16 fixed slots of 256 KiB each (4 MiB
  maximum static Image storage). Registration refuses atomically when full.
  Revocation still clears the exact entry after its final cap and loader pin
  retire. IDs remain monotonic generations, not slots.
- Raise the dynamic child budget to 24. This is an independent bound from the
  64 process records and supports 16 primary application instances plus
  bounded helper/process headroom. Spawn still refuses before allocation
  when this dedicated budget is full.
- Permit at most 128 aggregate 4 KiB PT_LOAD pages per dynamic Image. This
  bounds one executable's initial mapped image to 512 KiB, excluding its
  separately bounded stack and later process VM allocations. Existing ELF
  checks still enforce ET_EXEC, exact segment bounds, no W+X, strict user
  range, and an entry inside an executable segment.
- Add `SYS_IMAGE_INFO` for a caller-held exact dynamic Image/READ cap. It
  returns the kernel-validated entry point, lowest PT_LOAD base, and exact
  copied byte length. It does not mint, copy, spawn, or otherwise enlarge
  Image authority. Static boot Images remain outside this query and keep
  their existing Startup metadata.
- Desktop allocates one short-lived bounded shared staging region for an
  installed launch, sized to the 256 KiB file maximum. `packaged` reads the
  verified entry through a token-bound filesd operation, validates it via
  `SYS_IMAGE_REGISTER`, obtains its Image cap, and returns that cap to
  Desktop. Desktop obtains startup entry/base only from `SYS_IMAGE_INFO`,
  spawns, then releases staging memory and the transient Image source cap.
- The per-process image memory check grows to 128 pages only for dynamic
  Images. Process-wide VM accounting, heap commitment, and user threading
  remain separate resources and are not implicitly expanded by registration.

## Resource costs and refusals

The kernel reserves up to 4 MiB of static BSS for Image bytes plus small
fixed metadata. A 24-process dynamic workload may map at most 12 MiB of
PT_LOAD frames in aggregate before stacks and all other process resources;
actual frame availability remains the final admission limit. The package
manager uses one temporary 64-page staging region per serialized launch and
destroys it before returning. A launch above 256 KiB, an Image-table-full
state, a 129th PT_LOAD page, dynamic-child saturation, or any loader/ELF
refusal leaves no Image capability or child process.

No dynamic linker, ET_DYN, relocations, PIE, ASLR, Linux `mmap`, or POSIX
loader behavior is introduced. These remain a separate executable-format
decision.

## Required evidence

The guest proof must register and spawn a real installed APB1 ELF larger than
4 KiB, read its exact Image entry/base through `SYS_IMAGE_INFO`, and observe
its Process cap and ordinary window. Negative controls must cover over-size
Image bytes, an invalid ELF, a non-Image/wrong-rights query, table exhaustion,
stale Image metadata after revoke, and child-budget saturation. Resource
snapshots must show that the 4 MiB registry capacity is static kernel storage
and that the temporary Desktop staging region and Image references return to
baseline after launch.
