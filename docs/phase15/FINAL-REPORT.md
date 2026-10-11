# ArenaOS Phase 15 final report

Status: **PARTIAL / UNQUALIFIED**. Source is on
`arena/phase15-fpu-simd`, based on the merged Phase-14 mainline. ADR-0112
remains proposed. The branch is pushed for review; no PR, merge, or release was
created.

## What changed

The test suite now has a dedicated minimal ring-3 `#XF` executable. It uses
`-1.0f` as the `sqrtss` operand, clears all MXCSR sticky flags, unmasks the
invalid-operation exception, reads MXCSR immediately before the instruction,
and records both pre/post values if the instruction returns. The kernel
harness runs the fault candidate with a same-process sibling and an independent
peer, then reports statuses, scheduler-slot release, and resource-baseline
restoration even when the expected vector is absent. The test remains failed
when the instruction falls through.

The kernel ISA audit now parses executable PE section flags and instruction
bytes, requires gap-free decoded coverage of every executable section, rejects
empty disassembly, and retains its exact allowlist for six reviewed
state-management leaves. Negative controls cover missing instructions and
prohibited code. The canonical build checks that the dedicated probe was
produced.

## Root cause: SIMD exception failure

The probe is correctly configured: it executes the four-byte `sqrtss xmm0,xmm0`
instruction on `-1.0f`, and immediately reads MXCSR `0x1F00` before execution.
Both QEMU 11.0.2 TCG profiles return with MXCSR `0x1F01` and the fallback exit
status `0x00001f0000001f01`; they do not deliver vector 19 (`#XM/#XF`). The
same-process sibling and separate peer each exit 42, the fault slot is
released, and resource/thread counts return to baseline. `#MF` vector 16 and
`#UD` vector 6 containment pass on both profiles.

The diagnosis is supported by the matching QEMU 11.0.2 source: its TCG SSE
helper merges SoftFloat status into MXCSR but does not test the MXCSR exception
mask or raise vector 19. Its x87 path has a separate vector-16 raise. ArenaOS's
vector-19 IDT gate and no-error-code stub pass the live-IDT audit; `#MF` and
`#UD` exercise adjacent containment paths. A native host `SIGFPE` probe
corroborates that the same instruction faults on the host, but it does not
prove ArenaOS delivery.

The corrective change makes the stimulus and the failed condition
independently observable; it does not repair the external TCG exception path.
No test was masked, removed, or counted as passed. Until a suitable QEMU TCG
or hardware run delivers vector 19 to ArenaOS and passes containment, the
critical exception gate remains open.

## Root cause: Phase 14 reproducible build

The protected Phase-14 PIE hash mismatch was caused by a different target
standard library. The host used Rust 1.97.0 compiler binaries but had built
`x86_64-unknown-none` libraries from source. Phase 14's qualification used the
official Rust 1.97.0 rustup target components. The source-built sysroot
produced `2c3720dd…`; the verified official target component reproduced the
protected `d7b0a8cf…` identity. The source, lockfile, linker script and local
Rust flags were unchanged.

After replacing the target libraries and deleting stale generated Cargo
artifacts, the unchanged `tools/build.sh --image` completed. The protected
Phase-14 PIE remains `d7b0a8cf…`, the signed APB1 remains
`04add54f…`, and all normal build validations ran. No hash, signature, or
loader guard changed.

## Architecture and execution contract

The proposed baseline requires x87, FXSAVE/FXRSTOR, SSE and SSE2. Optional
SSE-family instructions remain CPUID-gated. FXSAVE preserves x87
control/status/registers, the legacy MMX-aliased state, MXCSR and XMM0–XMM15;
the guest fixture exercises x87 and SSE/SSE2 but does not claim an MMX
instruction regression. AVX/VEX, YMM upper halves, AVX2, AVX-512, ZMM and AMX
are unsupported. OSXSAVE is cleared after initialization; when inherited as
set, host policy tests verify XCR0 is reduced to `0x3` before OSXSAVE is
cleared. The tested OVMF boots entered with OSXSAVE clear, so that guest path
has no receipt.

The scheduler owns one 512-byte, 16-byte-aligned image per fixed slot. Source
review covered initialization before scheduler execution, bootstrap capture,
clean state before publishing a thread, the common save/restore path for
cooperative and timer-driven context switches, and safe reaping after moving
off a zombie stack. Slot reuse resets the image from the validated clean
template. Ordinary kernel code remains integer-only; the full executable
section audit checked 120,354 instructions and matched six narrow trusted
leaves.

## Results

- Canonical `tools/build.sh --image`: pass with protected Phase-14 PIE and
  signed APB1 identities.
- Kernel unit tests: 8/8 pass. Kernel and fixture rustfmt checks pass.
- ISA audit: 120,354 decoded instructions over the complete executable
  section; negative controls pass.
- Targeted QEMU runs: M1 8/8, M2 21/21, M3 13/13 and M4 9/9 pass on both
  profiles. Thread and process FP-state isolation, slot reuse, `#MF` and
  `#UD` containment pass. The `#XF` case fails on both profiles as described
  above; the UEFI guest shuts down cleanly after reporting the failure.
- Full historical suite, T2 10/10, T3 25/25, T4 100/100 and independent
  archive extraction/boot: not run because the focused `#XF` gate fails.
- Arena 1's SDL3 executable and Arena 2's runtime were not run or modified.

The tested source commit, EFI/ESP/fixture hashes, QEMU profiles, toolchain
identity and raw logs are recorded in [QUALIFICATION.md](QUALIFICATION.md) and
[`receipts/phase15-targeted-results.json`](receipts/phase15-targeted-results.json).

## Handoff

Arena 1 and Arena 2 may use the qualification record to review the proposed
x87/SSE state contract and compiler policy. They must treat it as experimental
until ADR-0112 is accepted and the `#XF` guest contract passes. No SDL3,
libc/libm, C runtime, graphics/runtime integration, or genuine upstream SDL3
compatibility is established here. See [Arena 1](ARENA1-HANDOFF.md) and
[Arena 2](ARENA2-HANDOFF.md).
