# Phase 15 thread-state isolation

Status: M and C accepted Phase 15 qualification for the pinned locally
modified QEMU 11.1.2 TCG backend. Strict `#XF` delivery passed on both CPU
profiles with containment and cleanup. The historical 120/120 diagnostic
aggregate remains separate, and the accepted result does not claim upstream or
hardware execution. ADR-0113 remains Proposed. See
[qualification](QUALIFICATION.md) and [the acceptance record](ACCEPTANCE.md).

## Ownership and transition rule

Each scheduler slot owns one private 512-byte, 16-byte-aligned FXSAVE64 image
alongside its saved stack context. The fixed 64-slot table costs 32 KiB plus
alignment padding and is kernel-only. Every context switch saves the outgoing
architectural state and restores the incoming image in the common assembly
leaf before the incoming saved register frame resumes. Interrupts are disabled
while the scheduler selects and commits the transition.

That path covers cooperative yield, IPC and synchronization blocking/wake,
timer preemption, process CR3 changes, thread exit, and first entry. Static
kernel context storage remains mapped across all process views, so the CR3 and
FP-state transition cannot be observed halfway through. Direct syscalls that
do not schedule leave user state untouched; ordinary kernel code is
integer-only.

## Initialization and cleanup

The clean image is validated before scheduler startup. The bootstrap slot and
new threads start from that image before execution. Reaped slots are cleared
only after switching off the zombie stack, then receive the clean template
before reuse. Process spawn/reap, joined and detached threads, forced cleanup,
and failed creation follow the same lifetime rule. Source review found no
restore from an uninitialized image or save into a freed/reused slot.

## Guest evidence

The ring-3 fixture checks x87 arithmetic and control/status, scalar SSE/SSE2
arithmetic, conversions and comparisons, MXCSR, XMM0–XMM15, and distinct MM0–MM7
patterns across timer preemption and cooperative yield. It crosses syscalls,
yield, a blocking join, timer preemption, separate process address spaces, and
slot reuse. Thread/process patterns remained isolated; the MMX fixture passed
with 12 timer switches, `EMMS`, and exact process teardown. The seeded
OSXSAVE diagnostic entered with XCR0 `0x7` and verified sanitation to `0x3`.
The M15 diagnostic reports 5/5 while explicitly saying `#XF NOT RUN`.

The strict `#XF` probe uses one faulting thread, a same-process sibling, and a
separate peer. On the pinned patched QEMU TCG backend, it delivered vector 19
with status `0x113`; the faulting thread terminated, sibling and peer survived,
the scheduler slot was reclaimed, resources returned to baseline, and the
kernel shut down cleanly. C independently replayed both profiles and inspected
the existing per-boot receipts. See [the acceptance record](ACCEPTANCE.md).

The diagnostic EFI/ESP still explicitly reports `#XF NOT RUN`; its 120/120
aggregate and earlier diagnostic stability remain separate from the strict
qualification. Official strict T2/T3/T4 passed on the immutable strict image
pair under the pinned patched backend.

## Independent review and qualification scope

- **SOURCE-REVIEWED** — the CPL3 fixture executes unmasked `SQRTSS` with
  MXCSR `0x1F00`; the kernel expects exit status `0x113` from vector 19.
- **HOST-TESTED** — the external kit pins the strict EFI, ESP, and compiled
  fixture hashes and checks the EFI embedded in the ESP.
- **LOCALLY-MODIFIED-TCG-GUEST-QUALIFIED** — the strict guest passed on both
  CPU profiles and official strict T2/T3/T4 passed 10/10, 25/25, and 100/100.
- **HARDWARE-BACKED-GUEST-TESTED** — not claimed; KVM or WHPX was not used.
- **NOT RUN** — Windows/WHPX guest execution and application-level SDL3/C
  runtime compatibility.

Run instructions and the receipt verifier are in
`tools/phase15_external/README.md`. A QEMU accelerator startup by itself does
not satisfy the `#XF` or state-isolation gate.
