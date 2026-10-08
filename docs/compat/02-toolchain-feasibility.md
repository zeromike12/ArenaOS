# 02 — C toolchain feasibility for ArenaOS

Status: experimental. Source of truth for every flag below:
`experiments/c-runtime/run.py` (`common_flags`, `LINK_FLAGS`, `COMPILERS`).
Nothing here changes the production build (`tools/build.sh`).

Evidence labels: **[STATIC]** from ELF/disassembly, **[GUEST]** executed
under ArenaOS, **[HOST]** host-only test.

## 1. Verdict

A **freestanding, static, non-PIE C program is feasible today** and has been
executed on ArenaOS under QEMU [GUEST, report 03]. Hosted libc is **not**
feasible without the work in report 05: there is no dynamic linker, no PIE
(Luna's Phase-14 scope), no POSIX file or socket model, and no C-facing
thread API. Nothing in this document duplicates Luna's ELF loader or PIE
work.

## 2. Target specification

| Item | Setting | Reason |
|---|---|---|
| Machine | x86-64, ELF64, little-endian | ArenaOS native target |
| Object type | static `ET_EXEC` | the only type `elf.rs` accepts (ADR-0108) |
| Base address | `0x200000` (text), data at next page | same window the other static guests use; placement belongs to Luna |
| Hosting | freestanding (`-ffreestanding -nostdlib`) | no libc, no crt1/crti/crtn, no libgcc |
| Floating point | none: `-mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387` | the kernel saves no FPU/SSE state and never sets CR4.OSFXSR (Phase-13 execution contract) |
| PIC/PIE | `-fno-pic -fno-pie -no-pie` | ADR-0108 |
| TLS model | `-ftls-model=local-exec` | fixed TP offsets; no `__tls_get_addr` |
| Stack protector / CET | `-fno-stack-protector -fcf-protection=none` | no runtime support for either |
| Unwind tables | `-fno-asynchronous-unwind-tables -fno-unwind-tables -fno-exceptions` | no unwinder in the image |
| Other | `-fno-builtin` (runtime defines its own mem/str), `-O2 -Wall -Wextra -Werror` | predictable code |
| Link | `-nostdlib -static -no-pie -Wl,--no-dynamic-linker -Wl,-z,noexecstack -Wl,--build-id=none -Wl,-z,max-page-size=4096 -Wl,-T,link/arena-user.ld` | strict subset the loader accepts |

The linker script (`experiments/c-runtime/link/arena-user.ld`) gives two
PT_LOADs (R+X text including `.init_array`, R+W data including `.tdata`,
`.tbss`, `.data`, `.bss`) and PT_TLS. It keeps W^X and defines
`__arena_tls_image/filesz/memsz/align` and `__init_array_start/end`.

## 3. Compilers

| Compiler | Version | Notes |
|---|---|---|
| GCC | 12.2.0 (Debian 12.2.0-14+deb12u1) | native host toolchain |
| Clang | 22.1.8 via `zig cc` 0.17.0 (`/opt/zig-clang/pkg/ziglang/zig`) | target `x86_64-freestanding-none`; zig's bundled LLD links |
| Binutils | readelf/objdump 2.40 | used by the audit |
| Host tests | GCC 12.2 with `-fsanitize=address,undefined` | glibc differential, host only |

No native `clang` binary is installed in the sandbox; `zig cc` provides
clang and LLD. A workstation with LLVM could use `ARENA_ZIG` to point at a
different driver, but the flag set has only been verified with zig.

## 4. Startup objects and runtime dependencies

- **Startup:** `src/crt0.c` provides `_start` (inline asm) and
  `arena_crt_start`. It switches to a 256 KiB VM stack, installs TLS, runs
  `.init_array` and calls `main`. There is no crt1.o, crti.o or crtn.o.
- **Runtime files (all compiled into each guest):** `crt0.c`, `rt.c`
  (exit, write, clock, yield), `alloc.c`, `string.c`, `stdio.c`, `vm.c`.
- **Runtime dependencies:** none beyond the syscall instruction. The link
  succeeds with `-nostdlib`, which means the object set resolves every
  symbol it needs (no libgcc helper was required for the probe).
- **ABI:** System V x86-64 inside the C program; ArenaOS native syscalls at
  the boundary (`arena_syscall6`, mirror of `userspace/abi.rs`).

## 5. Static results (ELF and disassembly)

Command: `python3 experiments/c-runtime/run.py --cc <cc> audit` (rebuilds
first). The audit mirrors `elf.rs validate()` and checks SSE/x87 operands
and x87 mnemonics.

| Build | Loader failures | SSE/x87 instructions | PT_TLS (vaddr, filesz, memsz, align) | `__arena_tls_*` agree |
|---|---|---|---|---|
| gcc 12.2 | none | 0 | 0x203000, 8, 24, 16 | yes |
| clang 22.1.8 (zig) | none | 0 | 0x204000, 8, 24, 16 | yes |

Both builds report `_start` at `0x200000`, entry inside the executable
PT_LOAD, no PT_INTERP, no W+X segment and no dynamic section. [STATIC]

## 6. Findings that changed the design

**Finding T1 — `-mgeneral-regs-only` did not stop zig clang emitting SSE.**
With the first flag set (`-mgeneral-regs-only -msoft-float`), `zig cc -S`
over the runtime sources and the probe (clang 22.1.8) produced **271 SSE
instructions** (`movaps`, `movdqa`, `movdqu`, `movups`, `pshufd`, and others).
The per-file counts were `stdio.c` 161, `alloc.c` 38, `crt_probe.c` 70 and
`vm.c` 2 (`string.c` 0). The breakdown was measured by compiling each file to
assembly and counting `%xmm`/`%mm` operands with a function attribution. It is
not attributed to a single function.

Two sources were separated by disabling the loop and SLP vectorizers
(`-fno-vectorize -fno-slp-vectorize`):

- **About 201 of the 271 (74%)** came from LLVM's auto-vectorization.
- **The remaining 70** remained with vectorization disabled. They are
  aggregate zeroing and copies (`movaps`, `movups`, `xorps`), lowered by the
  backend. In this toolchain, `-mgeneral-regs-only` does not prevent them.

Adding `-mno-sse -mno-sse2 -mno-mmx -mno-80387` removes the SSE target feature,
so both categories go to **0** for every runtime file and the probe. The audit
enforces this. The GCC build was not tested without the `-mno-sse` family, so
its SSE-free result with the current flags is observed but its
`-mgeneral-regs-only`-only behaviour is unknown. [STATIC, measured with `zig cc -S`]

**Finding T2 — the linker script under-reported TLS memsz.** The first guest
run failed T2 (`tls init=0x81 zero=23132`). Reading the ELF showed
`__arena_tls_memsz = 0x10` while PT_TLS `p_memsz = 0x18`. The script computed
`SIZEOF(.tdata) + SIZEOF(.tbss)`, which omits the 8 bytes of alignment padding
between `.tdata` and `.tbss`. crt0 therefore placed the thread pointer 16 bytes
too high. The fix is `ADDR(.tbss) + SIZEOF(.tbss) - ADDR(.tdata)`. The audit
now compares the symbols with PT_TLS. The negative control, the same objects
linked with the old script, fails with
`__arena_tls_memsz=0x10 but PT_TLS says 0x18`. [STATIC negative control,
GUEST before and after]

**Finding T3 — TLS in `.tbss` is not at offset 0.** In the gcc build
`tls_zero` sits at TLS offset 0x10 because `.tbss` is 16-aligned. That is
handled by the linker-computed tpoff; it is only correct because T2 was fixed.

## 7. Feasibility conclusions

- Freestanding static C: **feasible and executed** [GUEST].
- Clang/LLVM for ArenaOS: **feasible and GUEST-executed** with the flag set
  above. Its SSE behaviour needs the explicit `-mno-sse` family, verified by
  the audit.
- GCC for ArenaOS: **feasible and GUEST-executed**; no SSE was emitted with the same flags.
- Linker: the GCC builds link with the driver's default GNU ld (binutils
  2.40, bfd) and the zig/clang builds link with zig's bundled LLD. Both
  produce loader-acceptable ELFs by the static audit, and BOTH builds were
  executed on the guest under ArenaOS: the GCC/GNU-ld build and the
  clang/LLD build each printed `CRT-PROBE RESULT PASS (6/6)` (report 03).
- Hosted libc, PIE, dynamic linking, C++ runtime, threads: **not feasible in
  this milestone**; see report 05.
