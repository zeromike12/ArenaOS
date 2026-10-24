# QEMU TCG SIMD `#XF` source review

## Finding

The strict failure is in QEMU's TCG implementation of SSE floating-point
exceptions, not in the ArenaOS exception setup observed in the preserved guest
receipts. Across the reviewed QEMU 8.2.2, 10.0.13, and 11.1.2 source tags, the
`SQRTSS` helper records SoftFloat exception flags, but the TCG path has no
MXCSR-mask check that raises x86 vector 19 (`#XM`/`#XF`). The MXCSR update path
only copies those flags into the status word. This is the source-level reason
the unmasked invalid operation can return with MXCSR changed from `0x1F00` to
`0x1F01` and no vector 19.

For QEMU 8.2.2, the tag `v8.2.2` resolves to source commit
`11aa0b1ff115b86160c4d37e7c37e6a6b13b77ea` (tag object
`03e36c72c2c44f9afb116858d5e7c92be66a32b1`). In that source:

- [`target/i386/ops_sse.h`](https://github.com/qemu/qemu/blob/11aa0b1ff115b86160c4d37e7c37e6a6b13b77ea/target/i386/ops_sse.h#L552-L560)
  implements `helper_sqrtss` by calling `float32_sqrt(..., &env->sse_status)`;
  it does not inspect MXCSR masks or raise an exception.
- [`target/i386/tcg/fpu_helper.c`](https://github.com/qemu/qemu/blob/11aa0b1ff115b86160c4d37e7c37e6a6b13b77ea/target/i386/tcg/fpu_helper.c#L3069-L3091)
  ORs SoftFloat flags into `env->mxcsr`; `helper_update_mxcsr` only invokes
  that status update.
- [`target/i386/cpu.h`](https://github.com/qemu/qemu/blob/11aa0b1ff115b86160c4d37e7c37e6a6b13b77ea/target/i386/cpu.h#L1178-L1198)
  defines x86 exceptions through vector 18 (`EXCP12_MCHK`) and has no vector-19
  exception constant.

The same omission is present in the reviewed QEMU 10.0.13 source commit
`98f88efe8f07067667b8d9b1a5ce9551d34036c1` and QEMU 11.1.2 source commit
`4fc49f46dc95d4a27de2509e7fceb2931e91faeb`: their [`helper_sqrtss` bodies](https://github.com/qemu/qemu/blob/98f88efe8f07067667b8d9b1a5ce9551d34036c1/target/i386/ops_sse.h#L552-L560)
and [QEMU 11.1.2 body](https://github.com/qemu/qemu/blob/4fc49f46dc95d4a27de2509e7fceb2931e91faeb/target/i386/ops_sse.h#L552-L560)
also only call SoftFloat; their [MXCSR update helpers for QEMU 10](https://github.com/qemu/qemu/blob/98f88efe8f07067667b8d9b1a5ce9551d34036c1/target/i386/tcg/fpu_helper.c#L3268-L3289)
and [QEMU 11](https://github.com/qemu/qemu/blob/4fc49f46dc95d4a27de2509e7fceb2931e91faeb/target/i386/tcg/fpu_helper.c#L3284-L3300)
only merge flags, and their [QEMU 10](https://github.com/qemu/qemu/blob/98f88efe8f07067667b8d9b1a5ce9551d34036c1/target/i386/cpu.h#L1356-L1375)
and [QEMU 11](https://github.com/qemu/qemu/blob/4fc49f46dc95d4a27de2509e7fceb2931e91faeb/target/i386/cpu.h#L1488-L1507)
x86 exception constants stop at vector 18. QEMU 11.0.2 is covered by the
earlier source review in [`xf-tcg-analysis.md`](xf-tcg-analysis.md).

## ArenaOS contract review

The preserved QEMU 11.1.2 guest logs report `CR0=0x80010033` and `CR4=0x668`;
the latter includes both `CR4.OSFXSR` (bit 9) and `CR4.OSXMMEXCPT` (bit 10).
The FPU initialization code sets and reads those bits back in
[`fpu.rs`](../../../kernel/kernel/src/arch/x86_64/fpu.rs#L316-L325). The strict
fixture loads MXCSR `0x1F00`, checks it immediately before `sqrtss`, and uses
the negative finite operand `-1.0f` in
[`xf_probe.rs`](../../../userspace/phase15-fpu/src/xf_probe.rs#L11-L29).
The vector-19 IDT stub is a no-error-code exception stub in
[`idt.rs`](../../../kernel/kernel/src/arch/x86_64/idt.rs#L241-L245); the common
dispatcher sends a CPL3 fault to `exit_on_user_fault`, which records status
`0x113` and terminates the offending thread. The strict M15 test separately
checks that status, same-process sibling and independent-process survival, slot
reclamation, and resource restoration in
[`m15.rs`](../../../kernel/kernel/src/m15.rs#L449-L511).

These source and receipt checks do not show an ArenaOS defect. The clean
QEMU 11.1.2 TCG results already record the strict assertion failing while the
separate `#MF`/`#UD` containment and cleanup checks pass. No kernel workaround,
synthetic interrupt, or assertion change is justified by this evidence.
The existing recovery archive is
[`phase15-local-recovery-2026-10-21.tar.gz`](phase15-local-recovery-2026-10-21.tar.gz),
SHA-256 `fb1ff147b35373be554f0056930d9c7ddeb7ea00877d4a682ee19b17ec2a7033`.
Its QEMU 10.0.13 `max`/`qemu64` serial logs have SHA-256
`860e9eda888c03060224c260222918fcb5f75ab1a14fb2bb7eb41a572e927674` and
`c513d5ebb37eb6a91a1fbdc50cd973dace95003274676f46626b08148351e1b8`; the
QEMU 11.1.2 logs have SHA-256
`892dca2bd606fab6d79372e99ecae22aef7154d596877d6604369511c247ff9c` and
`a6788b217f19ed22013dbdb7b8d730ec4303448cb2d8eedd327aff0bf8f0693e`.
All four record `CR0=0x80010033`, `CR4=0x668`, pre-MXCSR `0x1F00`, post-MXCSR
`0x1F01`, M15 `FAIL (5/6)`, sibling/peer survival, exact cleanup, and clean
shutdown. Their QEMU process exits are 0 and semantic guest exits are 97, as
recorded in [`phase15-local-recovery.json`](phase15-local-recovery.json).

## QEMU 8.2.2 independent run

C reports that stock QEMU 8.2.2 booted the unchanged strict image on both
`max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`, then reported M15 `FAIL
(5/6)`: MXCSR changed from `0x1F00` to `0x1F01` and vector 19 was not
delivered. The reported checks for isolation, MMX, `#MF`/`#UD` containment,
survivors, resource cleanup, and clean shutdown passed. QEMU exited 0 while the
semantic guest result was 97. C also reports that explicit KVM initialization
failed because `/dev/kvm` was absent.

The reported evidence archive is
`ArenaOS-Phase15-Work-validation.zip`, SHA-256
`6b6aa2a4e88553a02a2ef707e415dd91b6e996268291fd177a0bf6d277fe591f`. That ZIP
was not present in this execution workspace, so its bytes and raw logs could
not be independently checked or copied into the committed receipt set. The
QEMU 8.2.2 source finding above was independently checked against the official
upstream tag and matches the result summary provided by C.

## Execution and qualification status

This execution environment has no `/dev/kvm`, QEMU executable on `PATH`,
Bochs, or PowerShell. `lscpu` identifies the outer hypervisor as KVM, but that
does not expose a nested KVM backend to this container. The repository's
onboarding QEMU file is a third-party musl build requiring
`/lib/ld-musl-x86_64.so.1`; it is not an official QEMU package and did not run.
No usable hardware-backed or independent emulator backend was available here.

No QEMU source patch was made. Fixing this TCG defect would require a separate,
architecturally complete implementation of unmasked SIMD exception delivery
and independent instruction-level validation; a result from such a locally
modified emulator would remain distinct from official Phase 15 qualification.
The existing strict image and acceptance test are unchanged. Phase 15 remains
**PARTIAL / UNQUALIFIED** pending a passing run on a trusted, unmodified
hardware-backed or independently validated backend and the remaining strict
stability gates.
