# Phase 15 qualification record

This record is intentionally partial. It preserves failed results and does not
claim the T2/T3/T4 or archive gates.

## Identity

- Base: merged Phase-14 `main`, `1aa35f4c20c9bb386612298fe21574897cf14159`.
- Branch: `arena/phase15-fpu-simd`.
- ADR: ADR-0112, proposed and pending architecture review.
- Tested source commit: `0182b5a0b980a2db0f39bcc41aa89ab500c6563b` (Phase-15 implementation; this evidence update is docs-only).
- EFI SHA-256: `6427d3a29cd11f4eb332c20698595fa98129bf0309fc83c29d75f3752c32d873`.
- ESP SHA-256: `5024c5e5c60f1484767653d925e549f6d3321765b6458ee9a8fc685a4d884964`.
- Embedded Phase-15 fixture SHA-256: `948ef296c2666a29a55d9b611de177452adffc54314a4843037bff3bb5304d2d`.
- Toolchain: rustc 1.97.0 (`2d8144b78`), Cargo 1.97.0 (`c980f4866`), LLVM/LLD 22.1.6.
- Guest runtime: QEMU 11.0.2 with TCG; OVMF EDK II. `/dev/kvm` is absent.
- Raw serial receipts: [`phase15-avx.log`](receipts/phase15-avx.log), SHA-256 `92f9d7c6c958e8b7a68543bda1f186b68f798e265c0b1e0bf9aa5d5cfe6f3c12`; [`phase15-qemu64.log`](receipts/phase15-qemu64.log), SHA-256 `58efd826ceb89f6ce1974bea35a59fe71abfa6c90d99185a25acee3ce4536c86`.

The EFI and ESP above were built from the Phase-15 source and fixture using a
direct UEFI-target build and `tools/espimg.py`. The canonical `tools/build.sh
--image` path fails before the kernel build: its Phase-14 static PIE guard
expects SHA-256
`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`, while
the locked fixture rebuild in this workspace produces
`2c3720dd4299dba89f95f78f16ca21fbd5a2f3a715a05ede6484a7d189c9b6dd`. The
guard was not bypassed and the expected hash was not changed.

## Evidence matrix

| Evidence | Result |
|---|---|
| Formatting | Pass: `cargo fmt --all -- --check` |
| Kernel host tests | Pass: 8/8 `arena-kernel` library tests |
| UEFI build | Pass: `cargo build --release --offline --target x86_64-unknown-uefi` |
| Phase-15 fixture build | Pass: release build for `x86_64-unknown-none` |
| CPUID and CPU controls | Pass on `qemu64` and `max`: required flags present; CR0 `0x80010033`; CR4 `0x668`; OSXSAVE cleared |
| Inherited OSXSAVE/XCR0 path | Partial: both OVMF runs entered with OSXSAVE clear (`XCR0=None`); host tests cover policy helpers, but no guest run entered with inherited OSXSAVE set |
| Clean FXSAVE state | Pass: startup validates FCW, status, tag, MXCSR/mask, zero XMM image, size and alignment |
| Ring-3 arithmetic and state | Pass in the worker fixture: x87, scalar SSE, SSE2, conversions, comparisons, x87 control/status and MXCSR |
| Syscall, yield, blocking and wake | Pass in targeted guest worker/joiner fixture |
| XMM0–XMM15 and x87/MXCSR thread isolation | Pass across yield, syscall, blocking join and timer preemption (39–42 switches per CPU profile) |
| Process isolation and slot reuse | Pass: distinct CR3 roots retained state; fresh process entered cleanly in a recycled slot; teardown exact |
| `#MF` and `#UD` containment | Pass on both profiles; sibling and separate process continued |
| `#XF` containment | Fail on both profiles. Unmasked `sqrtss` set MXCSR to `0x1F01` but returned to the guest fallback instead of delivering vector 19; the fault task returned `7937`, not expected `0x113`. The test did not misclassify this as a pass. |
| Kernel ISA audit | Pass: 119,490 real instructions, six exact trusted leaf profiles, ordinary x87/MMX/SSE/AVX rejected; negative controls pass |
| In-guest historical groups | M1 8/8, M2 21/21, M3 13/13, M4 9/9 passed in both targeted boots. This is not the complete historical regression suite. |
| Canonical image build | Blocked by the pre-existing Phase-14 fixture hash guard described above |
| T2 | Not run: targeted guest suite fails on `#XF` |
| T3 | Not run: targeted guest suite fails on `#XF` |
| T4 | Not run |
| Archive extraction and independent boot | Not run; no qualified candidate exists |

## Commands and receipts

Successful static checks:

```text
cargo fmt --all -- --check
cargo test -p arena-kernel --lib --target x86_64-unknown-linux-gnu
cargo build --release --offline --target x86_64-unknown-uefi
cargo build --release --offline                    # userspace/phase15-fpu
python3 tools/audit_kernel_isa.py --self-test
python3 tools/audit_kernel_isa.py --objdump <llvm-objdump> --image build/arena-boot.efi
```

Targeted guest command: `python3 tools/test_m15_guest.py`. It ran both
`max,+nx,+smep,+smap` and `qemu64,+nx,+smep,+smap`; both runs ended with
`m15: RESULT FAIL (2/3)` because the guest's unmasked SSE exception was not
delivered. The boot logs also preserve M1–M4 passing results, thread/process
isolation passes, `#MF` and `#UD` containment, MXCSR `0x1F01`, and clean UEFI
shutdown after the explicit failure. The raw serial receipts are committed
alongside this record; they include OVMF console control sequences.

This is observed on the available QEMU 11.0.2 TCG runtime. It is consistent
with missing `#XF` delivery in that execution environment, but no KVM or
hardware-accelerated cross-check was available here. Do not count either boot
as a Phase-15 pass.

## Gate decision

Phase 15 remains **partial and unqualified**. The state-preservation and
isolation tests passed, but the complete user-fault contract did not. T2/T3/T4
and independent archive verification must wait until `#XF` behavior is
validated on a suitable emulator or hardware and the canonical build is
reproducible. Preserve the failing test while diagnosing it; do not weaken the
expected exception or count the fallback status as containment.
