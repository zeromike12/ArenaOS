# C1 CPU FP/SIMD boundary audit (core-OS dependency for review with Luna)

Status: **audit and recorded dependency. No kernel FPU/SIMD code was written,
and no kernel file was changed by C1.** Review owner: Luna, after Phase 14.

Evidence labels used below: **STATIC** (source or disassembly, not executed),
**GUEST** (executed under ArenaOS). Nothing here is FP support for C.

## 1. Question

gcc and clang emit SSE/SSE2 for scalar `float`/`double` and for vectorised
loops on x86-64, and x87 for `long double`. ArenaOS must not let user code use
that state unless the kernel saves, restores, and enables it per thread. This
audit records what the kernel and the C prototype actually guarantee.

## 2. Findings

| # | Finding | Evidence | Label |
|---|---|---|---|
| F-FP1 | The kernel never saves or restores XMM, x87, MXCSR, or XSAVE state. A grep of `kernel/` (`*.rs`, `*.S`, `*.toml`) for `fxsave`, `xsave`, `xrstor`, `fninit`, `ldmxcsr`, `stmxcsr`, `mxcsr`, `fp_state`, `fpu`, `xmm`, `osfxsr`, `osxsave` matches only a comment in `arch/x86_64/context.rs` ("No FPU/SSE state is saved") and the `#MF x87 FPU Error` vector name in `arch/x86_64/idt.rs`. | source grep, re-run 2026-10-08 | STATIC |
| F-FP2 | Context switching is `switch_context` (`arch/x86_64/context.rs`). It pushes `rbx, rbp, rdi, rsi, r12`–`r15` and `pushfq`, and saves no FP/SIMD register. The thread record holds no FP state. | source read, `context.rs` `global_asm!` | STATIC |
| F-FP3 | CR4 is written only in `arch/x86_64/syscall.rs` (SMEP/SMAP enablement). That code reads the firmware-left CR4 value, ORs in SMEP and SMAP when the CPU reports them, and writes back. It never sets OSFXSR (bit 9), OSXMMEXCPT (bit 10), or OSXSAVE (bit 18). It does not assert PAE; PAE is whatever firmware left (the constant exists in `x86_64.rs`, but nothing checks it). It does not read or clear CR0.EM or CR0.TS. CR0 is written only to toggle WP in `paging.rs`. | source read: `syscall.rs` ~L522–L560, `paging.rs` ~L344, ~L1253 | STATIC |
| F-FP4 | Target feature baseline for `x86_64-unknown-uefi`. **Not verified in this session**: `rustc` is not on the sandbox PATH now, so the earlier `--print cfg` result is not re-run and is not relied on. | none | UNVERIFIED |
| F-FP5 | The built kernel image `kernel/target/x86_64-unknown-uefi/release/arena-boot.efi` (sha256 `b208d2c96eb13734fb9adf5a5aeb7a53e4a450e0b4d5ac391f8ec285063134a4`, identical to `build/arena-boot.efi`, 125 445 disassembly lines from `objdump -d`) contains **0** XMM/YMM/ZMM/MM register operands, **0** x87 mnemonics from the list in §5, and **0** `fxsave`, `xsave`, `xrstor`, `ldmxcsr`, `stmxcsr`, `vzeroupper`, or `emms`. | `objdump -d` re-run 2026-10-08 | STATIC |
| F-FP6 | Every guest ELF built by `experiments/c-runtime` has `simd_or_x87_count = 0` on the current tree: `c-native-gcc` (8 883 instructions), `c-native-clang` (7 770), `crt-probe-gcc` (5 386), `crt-probe-clang` (5 523). Loader audit failures: none. Compile flags: `-mgeneral-regs-only -msoft-float -mno-sse -mno-sse2 -mno-mmx -mno-80387`. `-mgeneral-regs-only` with `-msoft-float` alone did **not** stop clang emitting SSE; the `-mno-sse` family is required. | `run.py --cc gcc audit` and `--cc clang audit`, rc 0 on commit `76a55ee` | STATIC |
| F-FP6a | The audit rule was changed in `76a55ee`. The earlier rule flagged any mnemonic beginning with `f`, which matched data bytes decoded as text (`failed line %d:` in the executable segment) and failed the audit on string literals. The current rule matches an explicit x87/FP-state mnemonic list and keeps the exact register-operand rule (`%xmm`, `%ymm`, `%zmm`, `%mm[0-7]`, `%st`). The register rule is the precise SIMD check; the mnemonic list is a conservative backstop, not a full decoder. | `run.py` `X87_MNEMONIC`, commit `76a55ee` | STATIC |
| F-FP7 | GUEST runs of the C1 images never faulted and never showed FP output. Counts on the current tree: native app 5/5 exact PASS per compiler (`clang`, `gcc`; `summary-clang.json`, `summary-gcc.json`); crt-probe 3/3 PASS per compiler. Absence of a fault is consistent with the audit; it does **not** show that FP works. | `build/guest-native/summary-*.json`, `run.py guest` | GUEST (absence of fault only) |
| F-FP8 | The kernel has no FP/SIMD instructions in the built image (F-FP5) and no FP state save (F-FP1), so it does not depend on CR4.OSFXSR for its own operation. If a future kernel build emits SSE (a compiler flag, or a `core::arch` intrinsic), OSFXSR must be set explicitly and the context switch must save and restore FP state. That change must be reviewed. | F-FP1, F-FP5 | STATIC (conditional) |
| F-FP9 | Open: the firmware-left CR4.OSFXSR, CR4.OSXSAVE, CR0.EM, and CR0.TS values are never read or asserted. Today this does not matter because no code executes SSE/x87 in kernel or user images. It does matter for any future FP enablement, and for any firmware that leaves EM set. | F-FP3 | STATIC (gap) |

## 3. What this does NOT establish

- It does **not** provide floating-point support to C. A scalar `double` demo
  compiles to SSE on the default x86-64 target and would fault or corrupt
  state on this kernel. C1 makes no general FP claim, and the SDL3 handoff
  lists FP as unsupported.
- It does **not** test FP exception behaviour, MXCSR rounding, denormals, or
  x87 control words.
- It does **not** show that firmware leaves CR4.OSFXSR clear (F-FP9).
- It does **not** cover Blender's stated CPU baseline. Blender's requirements
  page lists 4 cores with SSE4.2 (see `04-compat-strategy.md`). That conflicts
  with the current no-SIMD contract. The conflict is recorded here, not
  resolved.

## 4. Required decision (for Luna's review after Phase 14)

Choose one of the following before any C application may use floating point:

1. **Keep the freestanding no-FP contract** (current state). The C toolchain
   stays free of SSE and x87; FP-using C code is refused at audit time. Lowest
   risk. It excludes SDL3's FP paths, Blender, and most game code.
2. **Enable FP with per-thread state.** Set CR4.OSFXSR and OSXMMEXCPT
   explicitly, use FXSAVE/FXRSTOR (or XSAVE if AVX is ever wanted) in the
   context switch, keep per-thread FP state in the thread record, initialise
   MXCSR, and extend the C audit to allow exactly the enabled set. This is a
   kernel change and needs its own ADR and review. It also resolves F-FP9.
3. **Enable FP without per-thread state.** Rejected: it leaks FP register
   contents across threads and processes.

Recommendation from C1 (not a decision): option 1 now. Option 2 becomes a
Phase-14-or-later ADR if SDL3 or a target needs FP.

## 5. Reproduction

```
cd experiments/c-runtime
python3 run.py --cc gcc audit      # rc 0; simd_or_x87_count 0 per ELF
python3 run.py --cc clang audit    # rc 0
objdump -d kernel/target/x86_64-unknown-uefi/release/arena-boot.efi > /tmp/kern.dis
grep -cE "%(xmm|ymm|zmm|mm)[0-9]*" /tmp/kern.dis                 # expect 0
grep -cE "\s(f(ld|st|ild|ist|add|sub|mul|div|com|ucom|xch|cmov|ninit|nstcw|ldcw|nstsw|wait|sqrt|abs|chs|nop)[a-z0-9]*)\s" /tmp/kern.dis   # expect 0
grep -cE "\s(ldmxcsr|stmxcsr|fxsave|fxrstor|xsave|xrstor|xsaveopt|vzeroupper|emms)" /tmp/kern.dis   # expect 0
sha256sum kernel/target/x86_64-unknown-uefi/release/arena-boot.efi
```

The x87 grep is a conservative mnemonic list, not an opcode decoder. The
register grep is exact for AT&T syntax as printed by `objdump`.
