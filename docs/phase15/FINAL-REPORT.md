# ArenaOS Phase 15 final report

Status: **partial; not qualified**.

Implementation is based on merged Phase-14 `main` at
`1aa35f4c20c9bb386612298fe21574897cf14159` and is committed on
`arena/phase15-fpu-simd`. ADR-0112 remains proposed, pending the architecture
review process. No PR, merge, or release was created.

## Implemented

- Explicit x87/FXSR/SSE/SSE2 discovery and checked CR0/CR4 setup, inherited
  OSXSAVE consistency checks, conditional legacy XCR0 reduction, and a
  validated clean x87/MXCSR/XMM template.
- Eager 512-byte, 16-byte-aligned per-thread FXSAVE64/FXRSTOR64 state for all
  64 scheduler slots, with clean initialization, retirement and reuse.
- Ring-3 arithmetic, thread, process, blocking, yield, timer-preemption,
  slot-reuse, and fault fixtures using the existing ET_EXEC loader.
- A fail-closed machine-instruction audit that allows only six exact reviewed
  CPU-state leaves and rejects ordinary x87/MMX/SSE/AVX instructions.
- Proposed ADR-0112, an ADR-0012 compatibility note, architecture/qualification
  records, and Arena 1/Arena 2 handoff notes.

## Verified evidence

- Kernel host tests: 8/8 pass; rustfmt check passes; UEFI target compiles.
- ISA audit: 119,490 genuine machine instructions checked; six trusted leaf
  ranges match; prohibited instruction controls pass.
- Targeted QEMU 11.0.2 TCG boots on `qemu64` and `max`: M1–M4 self-tests pass
  in both runs. Ring-3 thread isolation across yield, syscall, blocking join,
  and timer preemption passes. Separate CR3 process isolation and clean slot
  reuse pass with exact resource teardown. `#MF` and `#UD` containment pass.
- Both profiles read back CR0 `0x80010033` and CR4 `0x668`; firmware entered
  with OSXSAVE clear, and kernel setup kept it clear. `max` advertised AVX.

## Blocking failures and limits

- The unmasked SSE invalid operation leaves MXCSR at `0x1F01` and returns to
  the fixture fallback instead of delivering `#XF` vector 19. The guest test
  correctly fails with status `7937` instead of expected `0x113`. This occurs
  on both available QEMU TCG CPU profiles. No `/dev/kvm` is available for a
  hardware-accelerated cross-check. The `#XF` containment gate is therefore
  incomplete; the result is not attributed conclusively to the kernel or QEMU.
- `tools/build.sh --image` stops at the existing Phase-14 PIE reproducibility
  check. It expects fixture hash
  `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`, but
  the locked rebuild produces
  `2c3720dd4299dba89f95f78f16ca21fbd5a2f3a715a05ede6484a7d189c9b6dd`. The
  hash guard was not altered. A direct UEFI build and ESP image were made for
  targeted tests; that does not clear the canonical build gate.
- Guest evidence for inherited OSXSAVE with a live XCR0 sanitation path is
  absent; the tested OVMF starts with OSXSAVE clear.
- The complete historical suite, T2 10/10, T3 25/25, T4 100/100, and
  independent archive extraction/boot were not run. Arena 1's SDL3 executable
  and Arena 2's runtime were not modified or claimed compatible.

The exact tested artifact hashes, commands, receipts, and gate decisions are
recorded in [QUALIFICATION.md](QUALIFICATION.md). Phase 15 should remain
unqualified until the `#XF` test passes on a suitable execution environment,
the canonical build is reproducible, and all requested regression, stability,
and archive gates pass.
