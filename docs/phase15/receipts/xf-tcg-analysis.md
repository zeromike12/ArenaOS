# Phase 15 `#XF` investigation

## Guest instruction sequence

The dedicated `arena-phase15-xf-probe` ET_EXEC image is 4,744 bytes. Its
disassembly confirms the instruction bytes `f3 0f 51 c0` (`sqrtss xmm0,xmm0`)
and the preceding `movd xmm0,eax` with the value `0xBF800000` (`-1.0f`). The
probe loads MXCSR `0x00001F00`: all six sticky flags are clear and the invalid
operation mask is clear. It then reads MXCSR immediately before `sqrtss` and
requires exactly `0x1F00`. Integer-only instructions between this readback and
`sqrtss` cannot change MXCSR. If `sqrtss` falls through, the probe records both
MXCSR values in its 64-bit exit status.

The built guest reports the same result on both CPU profiles:

```text
status=0x00001f0000001f01, pre-MXCSR=0x00001f00, post-MXCSR=0x00001f01
```

The test therefore proves that the operand and unmasked control were in place
before the instruction, and that `sqrtss` set the invalid-operation sticky
flag before returning. The sibling in the same process and an independent
process both exit with status 42; the test still fails because the faulting
thread returned instead of receiving vector 19. `#MF` and `#UD` remain
separate passing containment checks.

## ArenaOS exception path review

- CPUID advertises FPU, FXSR, SSE, and SSE2. Kernel startup sets and reads back
  CR0.MP/NE, clears EM/TS, sets CR4.OSFXSR and OSXMMEXCPT, and clears OSXSAVE.
  Both guest logs show CR0 `0x80010033` and CR4 `0x668`.
- `kernel/kernel/src/arch/x86_64/idt.rs` installs `EXC_STUB 19, 0`: #XM/#XF
  has no hardware error code, and the stub supplies the synthetic zero used by
  the common frame contract. Vector 19 uses the same present, kernel DPL,
  64-bit interrupt-gate encoding as the other exceptions.
- The M1 live-IDT test reads IDTR and audits all 256 live gate selectors,
  attributes, IST values, and handler offsets. It passes in both raw boot
  receipts; vector 19's offset must equal its exception stub.
- The common exception entry passes vector, error code, and the hardware
  interrupt frame to the dispatcher. A CPL3 vector 19 is sent to
  `exit_on_user_fault(19)`, which records status `0x113`, terminates the
  offending thread, notifies process teardown, and schedules a survivor.
  The same path is exercised successfully by the guest `#MF` and `#UD` tests.

The ArenaOS IDT and containment code is architecturally consistent by source
review, but it cannot be credited with handling `#XF` until a real guest
delivery reaches vector 19.

## QEMU TCG root cause

The runtime is QEMU 11.0.2. The matching upstream source is tag `v11.0.2`
(annotated tag object `0ab45d2af85f2fc9885b526ffbc8a15bbd1a2da8`, peeled
source commit `e545d8bb9d63e9dd61542b88463183314cff9482`). In
[`target/i386/cpu.h`](https://github.com/qemu/qemu/blob/v11.0.2/target/i386/cpu.h),
the x86 exception constants stop at vector 18; there is no TCG `EXCP19` or
`EXCP_XM` constant. In
[`target/i386/tcg/fpu_helper.c`](https://github.com/qemu/qemu/blob/v11.0.2/target/i386/tcg/fpu_helper.c),
`update_mxcsr_from_sse_status` ORs SoftFloat status flags into `env->mxcsr`,
and `helper_update_mxcsr` only calls that status-update function. The TCG SSE
path has no check of the MXCSR exception masks and no vector-19 raise. The
separate `fpu_raise_exception` helper raises x87 `#MF` (vector 16), which is
why the existing x87 `#MF` test is not equivalent to SSE `#XF` delivery.

This source evidence matches the guest result on both `qemu64` and `max`.
Those runs use TCG; the host has no `/dev/kvm`. This is a QEMU TCG limitation,
not a result inferred only from the failed guest test. It does not excuse or
turn the failing guest assertion into a pass. Phase 15 remains unqualified.

## Host corroboration

[`fixtures/phase15-xf-host-probe.c`](../fixtures/phase15-xf-host-probe.c) is a
small Linux host probe. On the available x86-64 host it reports
`configured=0x00001f00`, `before=0x00001f00`, `sigfpe=1`, `si_code=7`
(`FPE_FLTINV`), and signal-frame MXCSR `0x00001f01`; disassembly confirms the
faulting RIP is the `sqrtss` instruction. This corroborates architectural
hardware behavior on the host. It is not evidence that ArenaOS handles the
exception.
