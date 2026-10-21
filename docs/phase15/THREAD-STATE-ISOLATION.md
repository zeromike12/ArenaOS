# Phase 15 thread-state isolation

Status: the uninterrupted historical diagnostic aggregate passed **120/120**
on source commit `989646771a0e288cefdaf0dabc5f5baa9bf1ed5c`. Thread/process
isolation, MMX state, fresh-slot cleanup, `#MF`, and `#UD` passed. Overall
Phase 15 remains **PARTIAL / UNQUALIFIED** because strict `#XF` delivery has
not passed. ADR-0113 remains Proposed. See
[qualification](QUALIFICATION.md) and the
[current local recovery receipt](receipts/phase15-local-recovery.json).

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
separate peer. QEMU TCG returns MXCSR `0x1F01` instead of delivering vector 19;
the guest records that failure while the sibling and peer exit 42, the fault
slot is released, and resources return to baseline. Those cleanup receipts do
not turn the `#XF` test into a pass. `#MF` and `#UD` containment pass separately.

The immutable diagnostic EFI/ESP completed T2/T3/T4 at 10/10 in 95 seconds,
25/25 in 237 seconds, and 100/100 in 952 seconds on QEMU 11.1.2 TCG. Each boot
exercised the diagnostic Phase-15 thread/process/MMX coverage while explicitly
reporting `#XF NOT RUN`; these counts do not satisfy official exception
qualification.

## External strict verification

- **SOURCE-REVIEWED** — the CPL3 fixture executes unmasked `SQRTSS` with
  MXCSR `0x1F00`; the kernel expects exit status `0x113` from vector 19.
- **HOST-TESTED** — the external kit pins the strict EFI, ESP, and compiled
  fixture hashes and checks the EFI embedded in the ESP.
- **TCG-GUEST-TESTED** — the latest QEMU 11.1.2 run, under both CPU profiles,
  shows the sticky invalid flag but no vector 19, so the strict case fails.
- **HARDWARE-BACKED-GUEST-TESTED** — **BLOCKED** until KVM or WHPX produces a
  passing guest receipt with sibling/peer survival and exact teardown.
- **NOT RUN** — official strict T2/T3/T4. KVM cannot initialize because this
  host has no `/dev/kvm`; the T2/T3/T4 results above are diagnostic-only.

Run instructions and the receipt verifier are in
`tools/phase15_external/README.md`. A QEMU accelerator startup by itself does
not satisfy the `#XF` or state-isolation gate.
