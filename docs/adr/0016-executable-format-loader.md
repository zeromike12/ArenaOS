# ADR-0016: Executable format and image loader

- Status: accepted (Milestone 4.1)
- Date: 2026-09-26
- Depends on: ADR-0001 (Rust/LLVM toolchain), ADR-0004 (no third-party
  runtime crates), ADR-0008 (kernel address space, W^X), ADR-0014
  (processes = address spaces), ADR-0015 (capability spaces)

## Problem

Milestone 4 needs real user programs. Before anything can execute, the
kernel needs (a) a decision on what bytes a program *is*, and (b) a
loader that validates those bytes and turns them into a populated
process address space. The image is untrusted input from the kernel's
point of view the moment images come from storage (Phase 5), so the
validator is a security boundary, not paperwork.

Constraints in force: static linking only (the scope firewall forbids
dynamic linking before Phase 8), W^X everywhere (ADR-0008), zero
third-party code in the image (ADR-0004), and our own toolchain
commitments (Rust + LLVM/lld, which are allowed dev tools).

## Approaches considered

1. **ELF64 container with our own strict-subset semantics.** Use the
   format rust-lld already emits for bare-metal targets; define exactly
   which parts the kernel honors and reject everything else.
2. **Bespoke flat format** (own magic, own header, own segment table).
   Full control, zero legacy — but we would also own a converter for
   every toolchain output forever (lld emits ELF; something must
   rewrite it), and we'd re-derive a worse version of a solved container
   problem for no runtime gain.
3. **PE/COFF** (what our UEFI boot chain already loads). Workable
   (lld emits PE for the UEFI target), but PE's semantics center on
   import tables and DLLs — exactly the dynamic-linking world we've
   firewalled — and its bare-metal static use is rarer than ELF's.
4. **Raw flat binary** (no metadata at all). No validation surface, but
   also no segment flags, no entry point, no BSS/memsz distinction —
   every one of those would have to be reinvented beside the format.

## Decision

**ELF64 as the container; the semantics are entirely ours** (option 1).
The kernel implements its own strict-subset validator and loader in
`kernel/kernel/src/elf.rs` — zero dependencies, manual little-endian
field reads, no struct casts over untrusted bytes.

A file format is not an OS lineage. ELF is a neutral System V container
used by the BSDs, seL4-based systems, Fuchsia, and Haiku; our own boot
chain already loads PE/COFF through UEFI without inheriting anything
from Windows. What ArenaOS inherits from ELF is *only* the container —
no Linux ABI, no GNU userspace conventions, no section semantics. The
program-header model happens to map 1:1 onto the loader's job:
segments become pages with flags.

### The ArenaOS ELF subset (the enforced contract)

An image is accepted iff **all** of the following hold; anything else
is a loud `Err`, never a shrug:

- `EI_MAG` = `\x7fELF`, `EI_CLASS` = ELFCLASS64, `EI_DATA` = LSB,
  `EI_VERSION` = 1, `e_version` = 1
- `e_type` = `ET_EXEC` — static images at fixed load addresses.
  (`ET_DYN`/PIE is deferred; it may return with Phase 8 packaging under
  a superseding ADR.)
- `e_machine` = `EM_X86_64`
- `e_phentsize` = 56; `e_phnum` in 1..=8 (`MAX_SEGMENTS`); the phdr
  table fully inside the file
- Per `PT_LOAD`: `p_vaddr` page-aligned; the whole
  `[p_vaddr, p_vaddr+p_memsz)` span in the canonical lower half
  (`< 0x0000_8000_0000_0000`); `p_memsz > 0`; `p_filesz <= p_memsz`;
  `[p_offset, p_offset+p_filesz)` inside the file; flags a subset of
  `PF_R|PF_W|PF_X`; **never `PF_W|PF_X` together** (W^X, ADR-0008 —
  the loader cannot create a writable+executable user mapping);
  segments pairwise non-overlapping (one VA maps once)
- `e_entry` lies inside a `PF_X` segment's VA span
- `PT_INTERP` present ⇒ rejected (dynamic semantics are refused
  outright, not ignored)
- All other program-header types are inert per the ELF rule that
  unknown types may be ignored — they cannot grant the loader any
  action, because only `PT_LOAD` consumes one. Section headers are
  never consulted: the program headers are the contract. Symbol and
  debug sections exist for *tools*, not for the kernel.

### Loader contract

- `elf::validate(bytes) -> Result<ImageInfo, &'static str>` — the
  subset check above; `ImageInfo` = entry + up to 8 `SegInfo`s.
- `elf::load(bytes, pid) -> Result<LoadInfo, &'static str>` — validate,
  then per segment page: refuse if the VA is already mapped in the
  target (double-load protection, checked *before* allocating),
  allocate a frame, zero it, copy the file-backed prefix
  (`p_filesz` portion; BSS is the zeroed remainder), map it with
  `paging::map_user_page_4k(root, va, frame, writable=PF_W, exec=PF_X)`.
- The mapped frames belong to the address space: `proc::destroy`'s
  user-half sweep frees leaves + tables + root, so load/destroy
  accounting is symmetric and test-exact (same ownership rule as
  ADR-0015's cap-mapped pages).
- Source-agnostic by construction: in M4.1 the image bytes are a
  compiled-in test artifact; in Phase 5 the same entry point receives
  bytes from the filesystem; in M4.5 spawn is gated by image
  capabilities. The loader never knows where bytes came from.

### The test image

`userspace/payload/` (`arena-payload`) — a genuine cargo/rust-lld
artifact for `x86_64-unknown-none` with our own `payload.ld`, laid out
at fixed addresses so tests assert exact bytes, not vibes:

| VA | Contents |
|----|----------|
| `0x200000` | `_start` entry stub (`jmp self` in M4.1; becomes the first real program — syscall debug-write + thread-exit — in M4.2/4.3) |
| `0x201000` | `META`: self-describing manifest (`ARENAOS-PAYLOAD!` magic, entry, BSS span) the tests cross-check against the parsed ELF header |
| `0x202000` | 4 KiB `.bss` (`NOLOAD` — proves zero-fill beyond `p_filesz`) |

Two `PT_LOAD`s: `0x200000` RX (flags 5), `0x201000` RW (flags 6).
`tools/build.sh` builds the payload before the kernel; the kernel
embeds it with `include_bytes!` (the kernel is its own only
"filesystem" until Phase 5).

## Consequences / disadvantages

- We own a parser for adversarial input. Mitigation: the m4 suite's
  rejection corpus — every rule above has at least one mutation test
  proving the rule fires (bad magic, wrong class/endian/machine/type,
  phdr-table overruns, W+X, overlap, unaligned vaddr, kernel-half
  vaddr, entry outside an X segment, `PT_INTERP`, zero-memsz, filesz
  overruns, truncation).
- ELF's flexibility invites scope creep (interpreters, dynamic
  sections, weird phdrs). Mitigation: the subset is closed by default —
  new acceptance requires a new ADR, and `PT_INTERP` is an explicit
  refusal rather than an omission.
- A perceived "Unix smell". Mitigation: this ADR — the container is
  pragmatism (our sanctioned toolchain emits it natively); every
  semantic rule above is ArenaOS's own, and several (W^X segments,
  ET_EXEC-only, aligned vaddrs, no interp) are *stricter* than what
  Unix systems accept.

## Implications

- `userspace/` comes alive with its first crate; the repo layout's
  "directories appear with code" rule is satisfied.
- New milestone suite `m4` (`m4:test:*`, `m4: RESULT`) +
  `tools/test_m4.py`; `run_tests.sh` picks it up via its glob.
- `paging::user_pte_flags` joins the API as the loader/tests'
  page-table introspection point.
- M4.2 (syscall ABI) and M4.3 (first executing user process) build
  directly on this loader: the payload's stub becomes the program, the
  `LoadInfo.entry` becomes the initial RIP.
