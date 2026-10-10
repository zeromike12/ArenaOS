# ADR-0110: Bounded static native PIE and kernel-selected placement

## Status

Accepted and qualified for the Phase-14 release. The exact production EFI
passed the 119-group historical/Phase-14 suite, fresh 100/100 stability, and
the independently extracted signed-APB1 QEMU witness. See
[`docs/phase14/FINAL-REPORT.md`](../phase14/FINAL-REPORT.md) for hashes and
receipts.

## Context

ArenaOS currently validates static ELF64 `ET_EXEC` images at declared fixed
addresses. Its verified APB1 launch path produces an exact immutable Image
capability, and process creation pins that Image through preparation. Startup
ABI v2 already transports the entry and image base in its one-page startup
record. There is no dynamic linker. Phase 14 needs to add one deliberate static
PIE format and randomized per-process placement without changing the package
authority model or the established `ET_EXEC` contract.

The existing VM arena begins at 32 TiB; existing mmap placements are below
1 TiB. The x86-64 lower canonical half extends to 128 TiB. `rngd` consumes the
virtio RNG after `DRIVER_OK` and has a readiness notification used by the
service manager. A PIE placement must not use a counter, time, caller input, or
fixed seed as entropy.

## Decision

### 1. Executable profile

Keep the current `ET_EXEC` checks and addresses unchanged for existing images,
including the established rule that unknown non-interpreter program-header
types are ignored and receive no loader semantics. Only the new `ET_DYN` path
interprets `PT_DYNAMIC`. Accept `ET_DYN` only as this bounded native profile:

- ELF64, little-endian, current ELF version, `EM_X86_64`, System V ABI, ABI
  version zero, zero reserved identification bytes, `e_flags == 0`, 64-byte
  ELF header, and 56-byte program header.
- At most 8 program headers and 8 `PT_LOAD` segments; the existing Image file
  limit remains 256 KiB and the mapped `PT_LOAD` page count and image span each
  remain at most 128 4-KiB pages.
- Program headers may be `PT_LOAD`, exactly one `PT_DYNAMIC`, and at most one
  `PT_GNU_STACK`. Reject `PT_INTERP`, `PT_TLS`, `PT_PHDR`, `PT_NOTE`,
  `PT_GNU_RELRO`, and every other program-header type. An optional
  `PT_GNU_STACK` must be empty and must not request execute permission.
- Each load segment has nonzero memory size, `p_filesz <= p_memsz`, read
  permission, `p_align == 4096`, page-aligned virtual address and file offset,
  and checked file and virtual ranges. Reject byte overlap and page-rounded
  overlap. The entry must fall in executable load memory. Reject every
  segment/page layout that would produce W+X. Segment flags are R, RW, or RX.
- `PT_DYNAMIC` is readable, non-executable, file-backed by a `PT_LOAD`, has
  equal file and memory sizes, `p_align == 8`, is no larger than one page, and
  terminates with exactly one `DT_NULL` at the end. Its bytes and all relocation metadata must resolve
  wholly into file-backed load memory. Reject duplicate, missing, or unknown
  dynamic tags.
- The permitted dynamic tags are exactly one each of `DT_FLAGS`, `DT_FLAGS_1`,
  `DT_DEBUG`, `DT_RELA`, `DT_RELASZ`, `DT_RELAENT`, `DT_RELACOUNT`,
  `DT_SYMTAB`, `DT_SYMENT`, `DT_STRTAB`, `DT_STRSZ`, and `DT_HASH`, followed
  by exactly one `DT_NULL` as the final record. `DT_FLAGS` must equal
  `DF_BIND_NOW`; `DT_FLAGS_1` must equal `DF_1_NOW | DF_1_PIE`; and `DT_DEBUG`
  must be zero. The symbol table must consist only of the all-zero null symbol
  (`DT_SYMENT == 24`); the string table must be one NUL byte (`DT_STRSZ == 1`);
  and the System V hash table must describe exactly that one symbol, with one
  zero bucket and one zero chain. No relocation may refer to a symbol.
  `DT_RELAENT` is 24; `DT_RELASZ` is a multiple of 24 and at most 12 KiB;
  `DT_RELACOUNT` equals the number of records. Dynamic table plus symbol/hash
  and relocation metadata is at most 16 KiB. A zero relocation count is valid
  only with a zero-sized table. All other dynamic tags—including dependency,
  PLT, text-relocation, TLS, and GNU-hash tags—are rejected.
- The only relocation is `R_X86_64_RELATIVE` (type 8) with symbol index zero.
  There are at most 512 entries. Each `r_offset` is 8-byte aligned and names
  one complete 8-byte destination in a writable, non-executable load segment.
  Each destination is unique. Each addend is a nonnegative link-time virtual
  address inside a load segment. No symbol lookup or external binding is
  performed.

The relocation formula is the x86-64 ELF `RELATIVE` operation: write `B + A`
to `B + r_offset`, with checked arithmetic, where `B` is this launch's load
bias and `A` is the signed `r_addend` (negative values are rejected by this
profile). The destination address and value must remain in the lower canonical
user half and the destination must meet the writable non-X rule above.

The fixture uses Rust 1.97.0's `rust-lld`, a checked-in crate-local Cargo
configuration, `-pie`, `--no-dynamic-linker`, `--no-undefined`, and
`--hash-style=sysv`. The null-only symbol/hash metadata above is the bounded
profile emitted by this linker; it enables no symbol binding. The kernel does
not broaden acceptance to accommodate unrelated linker metadata. Linker output
must be inspected and recorded before the fixture is qualified.

### 2. Placement and address ownership

- Choose a per-launch 2-MiB-aligned load bias `B` from the half-open interval
  `[0x0000_4000_0000_0000, 0x0000_6000_0000_0000)` (64–96 TiB). This provides
  2^24 candidate slots before subtracting image-size boundary constraints.
- The lowest link-time load address may be zero. The actual Startup ABI image
  base is `B + lowest_PT_LOAD_p_vaddr`; it is not necessarily the bias.
- The complete page-rounded image envelope, one unmapped 4-KiB guard page
  below it, and one guard page above it must fit in the placement arena.
  Place the existing bounded user stack after the upper image guard with its
  established stack guard. Keep Startup ABI and other fixed mappings in their
  existing locations.
- Preflight the selected envelope against the process mapping inventory and
  page tables before any image page is populated. Retry a bounded number of
  independent candidate slots when a collision occurs; report placement
  exhaustion as a typed refusal. Do not accept a placement address from the
  Image or application.
- Register process ownership for the complete image span, including unmapped
  holes. Leave both image guards absent from the page tables. Process teardown
  must reclaim all mapped image/stack pages, tables, and ownership records.

### 3. Entropy source and readiness

- `rngd` obtains 32 bytes from the virtio RNG only after the device reports
  `DRIVER_OK`. It passes them to a new narrow kernel entropy-seed syscall using
  a dedicated exact `KernelEntropySeed` capability. That capability is given
  only to the production rngd bootstrap, has no-copy/write-only semantics, and
  cannot be duplicated or delegated through the ordinary capability API.
- The kernel seeds its ChaCha20 CSPRNG from those 256 bits. For each PIE spawn,
  it draws candidate slots with rejection sampling to avoid modulo bias. A
  refill/reseed may mix fresh rngd bytes. The service manager is notified ready
  only after successful kernel seeding. No fixed-address boot service waits on
  this interface.
- ELF structural validation does not depend on RNG readiness. If the entropy
  source has not seeded the kernel generator, a seed request fails, or
  candidate selection is exhausted, the PIE spawn-check/spawn operation
  returns a typed no-entropy/placement error. There is no clock, counter, or
  deterministic fallback. `ET_EXEC` remains launchable.
- Trust is rooted in the virtio RNG device and its backend plus the trusted
  rngd/kernel boundary. A malicious or defective device/backend can reduce
  entropy. The selected arena has 24 bits of placement choice for the maximum
  2-MiB slot grid; smaller images have no more than that and boundary exclusions
  reduce the count. This is address-space randomization, not a claim of 24 bits
  of total system security or protection against memory disclosure.

### 4. Load, protect, publish, and rollback

For PIE, use this sequence: validate all metadata; obtain random placement;
preflight the full image, stack, guards, and reserved ranges; reserve process
ownership; populate private pages writable and non-executable while no user
thread exists; apply the validated relative relocations; set final page
protections; write the actual relocated entry and image base into the
ABI-v2-defined fields of the exact one-page Startup ABI SharedRegion; then
publish and start the process.

Final protections follow segment flags: executable code is RX, read-only data
is R, writable data and BSS are RW, and no user page is simultaneously W and
X. Relocation writes are restricted to destinations checked by validation.
Image guards and the stack guard have no PTE. The kernel may use private
temporary mappings but never exposes a writable-and-executable page to ring 3.

The Startup ABI record remains version 2. For a zero-based `ET_DYN` image,
Desktop first encodes the valid template pair `base = 0x0040_0000` and
`entry = base + link_time_entry`; this is only a codec-valid private template,
not a placement request. Kernel patching is limited to the existing 64-bit
entry and image-base fields at offsets 104 and 112. It checks the exact
one-page startup SharedRegion authority and the expected link-time/template
pair, then writes the actual relocated entry/base before the child thread can
run. Existing descriptor inventory and startup transport checks remain
userspace-enforced. For `ET_EXEC`, the current static base/entry behavior is
unchanged.

Any failure after pinning destroys the unpublished process/address space,
clears its spawn record, drops the Image pin, and reclaims provisional caps,
image/stack pages, child page tables, and ownership records before returning.
No failed launch leaves a live child or executable mapping. The existing
SharedRegion unmap contract retains empty parent page-table frames in the
long-lived broker address space for reuse; this is not a mapping, pin, region,
or child-owned frame. A one-page Startup mapping can warm up at most three such
tables on the first launch. The same immutable Image capability may be pinned
for multiple launches, with independent bases and process-owned memory.

## Consequences

- ArenaOS supports a narrow static PIE profile and `R_X86_64_RELATIVE` only;
  there is no loader, symbol resolution, shared library, PLT binding, TLS, or
  Linux executable ABI.
- Per-launch base and entry are process state, never Image-registry metadata.
- PIE launch requires a real RNG-backed kernel seed; existing `ET_EXEC`
  services do not.
- The 64–96-TiB arena is disjoint from the current 1-GiB–1-TiB mmap and 32-TiB
  VM arena. The bounded image span and guards leave the kernel half untouched.
- Static policy checks do not replace guest evidence. The release must record
  actual linker output, actual selected bases and range, positive and malformed
  signed APB1 launch results, exact cleanup, historical regressions, and
  exact-artifact boot qualification.

## Qualification evidence

- A genuine linker-produced `ET_DYN` Rust executable with executable code,
  read-only constants, initialized writable data, BSS, a relative relocation,
  and observable successful exit.
- Production-validator positive and mutation-negative host tests.
- QEMU proof that a signed installed APB1 PIE app runs through the protected
  filesd/packaged/Image path, sees correct relocated state and actual Startup
  base/entry, exits, and runs again at a distinct valid base.
- Signed negative cases for malformed ELF/relocations, unsupported relocation,
  out-of-image target, arithmetic overflow, bad alignment, W+X, protected-range
  collision, missing entropy, stale/revoked Image, and resource exhaustion.
- Guard, W^X, teardown, Image pin/refcount, and process-capacity receipts.
- Full historical plus Phase-14 suite: 119/119 groups passed.
- Exact final EFI SHA-256: `418c63acbf9eee0eadb6dd8de21e860bddd5e868d612028d7e93cb45e8d1ff2f`;
  fresh stability passed 100/100 boots against that artifact.
- Independently extracted archive boot verified archive checksums, Phase-13
  fixed-address applications, and two signed installed PIE launches with
  distinct bases and exact teardown.
