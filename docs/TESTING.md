# ArenaOS — Testing Strategy

Governing rule (ADR-0005): **a subsystem exists only when a test would fail
if it were faked.** "Printing success" is not a test. Tests assert observable
machine effects: bytes that came back out of a UART in loopback mode, register
values read back after a GDT load, firmware structures parsed from real UEFI
memory, privilege-level changes, VM exit behavior.

## The testing pyramid

```
        ┌────────────────────────┐
        │ in-guest self-tests    │  hardware behaviors: UART loopback,
        │ (m<N>:test:* markers)  │  sgdt read-back, #PF traps, ring transitions
        ├────────────────────────┤
        │ boot integration tests │  build → fresh ESP → QEMU/EDK2 → serial
        │ (tools/test_m<N>.py)   │  marker assertions → clean-exit check
        ├────────────────────────┤
        │ host unit tests        │  arch-independent logic: allocators, rings,
        │ (cargo test --target   │  parsers, data structures
        │  x86_64-unknown-linux) │
        └────────────────────────┘
```

## Marker grammar (serial)

Every milestone emits machine-checkable lines:

```
m1:test:<name>: PASS
m1:test:<name>: FAIL (<reason>)
m1: RESULT PASS (8/8)
m1: RESULT FAIL (6/8)
m2:test:<name>: PASS                        ← same grammar per milestone
m2: RESULT PASS (3/3)
[arena PANIC <file>:<line>] <message>       ← any panic fails the run
```

The harness (`tools/test_m<N>.py`, a thin wrapper over the shared pipeline in
`tools/mtest.py`) requires: all expected test names present with PASS, RESULT
line PASS *and covering exactly the expected test count*, no PANIC anywhere,
QEMU exits by itself (the kernel calls UEFI `ResetSystem(EfiResetShutdown)`;
`-no-reboot` makes QEMU exit 0) within the timeout. A timeout means hang; a
nonzero QEMU exit means crash/reset-loop — distinct diagnoses. All milestone
harnesses run against the same image in one boot: the kernel executes every
milestone suite in order, so M1 markers prove the older guarantees still hold
while M2 code is present.

### Exception-path testing (M2.1+)

Exception tests use *real* faulting instructions (divide-by-zero, writes to
unmapped memory) and must survive them: the suite arms an expected vector
(`arch/x86_64/faults.rs`), the fault site records its own resume address,
the handler verifies/recovers, and the test asserts the **measured** delivery
(vector, error code, CR2) — never the expectation. Unarmed faults still take
the full diagnostics-and-halt path, so injection cannot mask real bugs.

**Standing invariant:** `0x0000_6000_0000_0000` (m2.rs
`UNMAPPED_CANONICAL_ADDR`) must remain unmapped in *every* address space this
project ever builds — firmware's, the M2.4 kernel tables, and any later user
address space. It is the #PF test's faulting address.

## Determinism

- Fixed machine (`q35`), fixed RAM (512M), fixed CPU model (`qemu64,+nx`),
  fixed firmware blobs (pinned QEMU package).
- No wall-clock assertions; time sources are treated as monotonic counters.
- Scheduler tests use an explicit deterministic round-robin mode (M3).
- Fresh OVMF_VARS and fresh ESP image every run (no state survives between
  runs).

## Regression discipline

- Milestone test scripts are never deleted; `tools/run_tests.sh` runs *all*
  of them. Green means: every milestone ever completed still completes.
- Any change that makes an old test flaky is treated as a kernel bug until
  proven otherwise.

## Case study: the M1 timer test (post-mortem, kept as a teaching artifact)

`m1:test:timer_irq_absorbed` went through three failure modes before it was
deterministic; each taught a rule now baked into this document.

1. **Triple fault on the very first interrupt.** Our `IdtGate` struct encoded
   the type/attribute byte at the wrong offset (a packed-layout mistake), so
   every gate looked non-present/ill-typed to the CPU: `#GP(vector*8+2)` →
   `#DF` → triple fault, with *no* serial output. Rule: **tests must assert
   architectural byte layouts read back from hardware** — `test_idt_installed`
   now audits all 256 live gates (selector, attribute, IST, handler offset)
   against independently computed expectations, never against the same struct
   that wrote them (a self-consistency audit would have passed).
2. **Flaky single-tick pass/fail.** Probes showed firmware hands off with both
   8259 IMRs at 0xFF and the PIT tick routed IOAPIC → LAPIC. Our stub EOI'd
   only the 8259, so the LAPIC in-service bit stayed set: exactly one tick
   could ever be absorbed per boot, and the test passed only when that tick
   landed after the `before` snapshot. Rule: **a test that can pass on a
   single lucky hardware edge is not a test** — it now requires *two*
   absorbed ticks, which only happens if the full
   deliver → absorb → EOI → re-deliver cycle works.
3. **A #DE "divide error" from an instruction with no division.** An interim
   fix unmasked 8259 IRQ0; because firmware never programmed the 8259 vector
   base (irq_base=0), the ExtINT acknowledge delivered *vector 0* — our own
   gate-0 diagnostic stub, reporting a #DE at a plain `mov` load. The
   diagnostic stub earned its keep (vector, error code, RIP, CS, RFLAGS all
   on serial), and RIP-to-source mapping worked without symbols by anchoring
   the runtime image base via the printed GDT address and disassembling the
   PE at the computed offset. Rule: **probe the machine, not the manual** —
   firmware handoff state (both interrupt controllers, masks, vector bases)
   is a measured fact in this repo, documented in `ARCHITECTURE.md`.

Known quirk (not a bug): the `total_described=` MiB figure in
`m1:test:memory_map` exceeds installed RAM because firmware memory-map
descriptors include huge reserved MMIO spans (64-bit PCI window); the RAM
figures are `conventional=`/`reclaimable=`.

## Adding tests for a new milestone

1. Define markers in the kernel (`mN:test:<name>`), each backed by a real
   assertion path (no `assert!(true)` theater).
2. Add `tools/test_mN.py` modeled on `test_m1.py` (shared helpers live in
   `tools/qemu_env.py` / `tools/espimg.py`).
3. Wire it into `tools/run_tests.sh`.
4. Document exit criteria in `docs/ROADMAP.md`.

## Future layers (scheduled, not speculative)

- **Fault injection** (M2+): deliberate exceptions, corrupted structures fed
  to parsers, double-frees against debug allocators.
- **Stress loops**: 100-boot stability loop for the ExitBootServices
  transition (M2.7); allocator churn tests.
- **Host fuzzing** (Phase 5+): boot-info parser, FS metadata parser, image
  loader — all reachable from host unit-test binaries.
- **KVM acceleration** when the host allows it; test semantics unchanged.
- **Packet capture assertions** (Phase 7): QEMU slirp/tap + tcpdump-grade
  checks that bytes actually went on the wire.
