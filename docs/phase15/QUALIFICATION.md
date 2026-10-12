# Phase 15 qualification record

Status: **PARTIAL / UNQUALIFIED**. The canonical build and focused state
isolation tests pass. The required unmasked SIMD `#XF` guest test fails on both
available QEMU TCG profiles. The full historical suite, T2, T3, T4, and
independent archive verification were not run and are not claimed.

## Source and artifact identity

- Repository: `zeromike12/ArenaOS`.
- Branch: `arena/phase15-fpu-simd`.
- Starting point: merged Phase-14 mainline `1aa35f4c20c9bb386612298fe21574897cf14159`.
- Tested source commit: `ad87bee8d199b285ca1a2f3a31c5dbdcf4994faa`. This is the code/test commit; subsequent changes in this documentation commit do not alter build or test inputs.
- Canonical command: `tools/build.sh --image`, using official Rust 1.97.0 `x86_64-unknown-none` and `x86_64-unknown-uefi` target components.
- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`, Cargo `1.97.0 (c980f4866 2026-06-30)`, LLVM/LLD `22.1.6`.
- `rustc` SHA-256: `df13f58759c0662831983e3a6501c63c1fc12ea60ec4e1d1ac35e5fe43c500c0`; `rust-lld`: `8e5130a02d8f3473ed4c4b971d43c301205c89854bb467257e64eafe4bf21ca8`.
- QEMU: `11.0.2`, TCG. Recorded QEMU executable SHA-256: `0539587908d9a9b83ccdce7a48d968b50f827476b9852814e07445e9402df562`; matching upstream source tag `v11.0.2`, peeled commit `e545d8bb9d63e9dd61542b88463183314cff9482`.
- `/dev/kvm` is absent. No KVM-backed claim is made.

| Artifact | Size | SHA-256 |
|---|---:|---|
| Canonical EFI `build/arena-boot.efi` | 11,029,504 bytes | `04d285b3489fa9d1365c767677c4e98baef91b634c203f5735118f0593b83b1a` |
| Canonical ESP `build/arena-esp.img` | 16,777,216 bytes | `dc9d09ccbc62bed566459c70673a2d5e4dd1a18d468f877508dacf9fdceee326` |
| Phase-15 state fixture | 8,824 bytes | `786808ad21686ace59684f01ed1e69b47bd104c53442b2d7df9e646b9103b32b` |
| Standalone `#XF` probe | 4,744 bytes | `1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3` |
| Protected Phase-14 PIE | 23,256 bytes | `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e` |
| Signed Phase-14 APB1 | 24,040 bytes | `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa` |

Machine-readable run metadata is in
[`phase15-targeted-results.json`](receipts/phase15-targeted-results.json).
The complete canonical build log is
[`phase15-canonical-build.log`](receipts/phase15-canonical-build.log),
the harness summary is
[`phase15-targeted-run.log`](receipts/phase15-targeted-run.log), and raw serial
logs are [`phase15-avx.log`](receipts/phase15-avx.log) and
[`phase15-qemu64.log`](receipts/phase15-qemu64.log). Each receipt's SHA-256 is
listed in the JSON record.

## Results

| Gate | Result |
|---|---|
| Rust formatting | Pass: kernel workspace and Phase-15 fixture checks |
| Kernel host tests | Pass: 8/8 |
| Canonical build | Pass: `tools/build.sh --image`; protected Phase-14 PIE hash and signed APB1 identity unchanged |
| Kernel instruction audit | Pass: 120,354 decoded machine instructions across the full executable section; six trusted state-management leaf profiles; zero-instruction and prohibited-instruction negative controls fail as required; the audit rejects gaps in actual executable-section coverage |
| Phase-15 feature/template policy | Pass in host tests; exact clean FXSAVE size/alignment, MXCSR mask handling, required feature checks, and fail-closed inherited OSXSAVE/XCR0 policy checked |
| QEMU CPU initialization | Pass in both profiles: CR0 `0x80010033`, CR4 `0x668`, OSXSAVE clear. OVMF entered with OSXSAVE clear, so guest XCR0 sanitation was not exercised |
| Historical in-guest milestones | M1 8/8, M2 21/21, M3 13/13, M4 9/9 in each targeted boot. This is not the full historical regression suite |
| Thread FP-state isolation | Pass in both profiles across x87/MXCSR/XMM0–XMM15, syscalls, yield, blocking join, and timer preemption |
| Process isolation and scheduler-slot reuse | Pass in both profiles; clean fresh process state and exact teardown receipts |
| `#MF` and `#UD` containment | Pass in both profiles; faulting thread exited while sibling and independent process survived |
| `#XF` containment | **Fail in both profiles**: unmasked `sqrtss` read MXCSR `0x1F00` immediately before execution, fell through with `0x1F01`, and returned fallback status `0x00001f0000001f01` instead of `0x113`. Sibling/peer survival, slot release, and baseline restoration passed; they do not turn the exception gate into a pass |
| T2: 10 fresh boots | Not run; focused suite has a failed required gate |
| T3: 25 mixed-workload boots | Not run |
| T4: 100 fresh boots of one artifact | Not run |
| Full historical regression | Not run; only M1–M4 results above are included |
| Independent archive verification | Not run; no qualified release candidate exists |

The targeted runner attempted one boot for each profile, with no retries. Both
QEMU processes shut down cleanly through UEFI after the guest reported its
failure. The test harness returned semantic failure code 97 for each boot;
QEMU's own process return was zero. These are failed targeted qualification
runs, not passing boots.

### CPU profiles and `#XF` evidence

The profiles were `max,+nx,+smep,+smap` and
`qemu64,+nx,+smep,+smap`. The `max` profile advertises AVX, and `qemu64` does
not; the kernel cleared OSXSAVE in both. Each run reported the same exact
probe status, with the same-process sibling and separate-process peer exiting
42, the fault slot released, and frame/thread counts restored to baseline.

The probe ELF disassembles to bytes `f3 0f 51 c0` (`sqrtss xmm0,xmm0`) with
operand `0xBF800000` (`-1.0f`). It writes MXCSR `0x1F00` (sticky flags clear,
invalid mask clear), reads it back immediately before the operation, and
returns the recorded pre/post values if execution falls through. The fixture
therefore rules out stale flags, a masked invalid exception, an optimized-away
instruction, and a changed MXCSR setup between the readback and instruction.

The QEMU 11.0.2 TCG source inspection is recorded in
[`xf-tcg-analysis.md`](receipts/xf-tcg-analysis.md): the TCG SSE helper merges
SoftFloat exception flags into MXCSR but does not check the exception mask or
raise vector 19; the x87 helper separately raises vector 16. This source-level
finding matches both guest runs. The live IDT audit also passes and checks
vector 19's no-error-code stub and gate. The host `SIGFPE` probe corroborates
native host behavior only; it is not evidence of ArenaOS `#XF` delivery.

### Phase 14 reproducibility recovery

The initial `2c3720…` PIE difference came from the target standard library,
not from a changed Phase-14 source, linker script, lockfile, signature, or hash
guard. The workspace had built `x86_64-unknown-none` libraries from the Rust
1.97.0 source tree, while Phase 14 recorded official rustup target components.
The official Rust manifest target component archives matched their published
hashes; installing those components into the same compiler restored the
protected `d7b0a8…` PIE. After removing stale Cargo target directories,
`tools/build.sh --image` passed unchanged. The signed APB1 remains at
`04add54f…`. Details and component hashes are in
[`phase14-pie-recovery.md`](receipts/phase14-pie-recovery.md). No reference
hash, signature check, loader check, or build guard was weakened.

## Supported execution policy and lifecycle review

The proposed guest baseline requires x87, FXSAVE/FXRSTOR, SSE, and SSE2. The
64-bit FXSAVE image preserves x87 control/status/registers, MMX-aliased legacy
state, MXCSR, and XMM0–XMM15. Additional SSE-family instructions are usable
only when the processor advertises them through CPUID; this proposal does not
make SSE3, SSSE3, SSE4.1/4.2, or SSE4a unconditional. The fixture exercises
x87 and SSE/SSE2; it does not claim a guest MMX-instruction regression.
AVX/VEX, YMM upper halves, AVX2, AVX-512, ZMM, and AMX state are unsupported.

The implementation review checked 64 exact 512-byte, 16-byte-aligned
slot-owned images; initialization before scheduler startup; bootstrap capture;
clean state before a thread becomes Ready; one common save/restore leaf for
cooperative, blocking, process, exit, and timer switches; and retirement only
after switching away. Reaping clears the image before slot reuse. Kernel
ordinary code remains integer-only. The ISA audit permits six exact reviewed
state-management leaf profiles and rejects ordinary x87/MMX/SSE/AVX code.

The tested firmware started with OSXSAVE clear. Host tests cover the
conditional inherited-OSXSAVE/XCR0 policy, but no guest receipt demonstrates
that path on these boots.

## Gate decision

Keep Phase 15 **PARTIAL / UNQUALIFIED**. The test now isolates the actual
instruction and records independent failure/cleanup evidence, and the canonical
build blocker is resolved. The `#XF` guest contract still fails under the
available QEMU TCG, so T2/T3/T4, full historical qualification, and archive
verification remain unrun. ADR-0113 stays proposed; do not claim production
support or SDL3 compatibility from this record.
