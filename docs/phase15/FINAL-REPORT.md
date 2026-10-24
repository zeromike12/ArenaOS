# ArenaOS Phase 15 qualification acceptance

Phase 15 qualification was **accepted by M and C on 2026-10-24** for the pinned
locally modified QEMU 11.1.2 TCG backend. ADR-0113 remains Proposed as a separate
architecture decision. The accepted scope is not upstream-QEMU or hardware-backed.
See the [acceptance record](ACCEPTANCE.md).

## Results

- Expanded independent SSE/SSE2 exception controls passed on the patched build
  and failed expected exception cases on the same-source unpatched build.
- The unchanged strict guest passed 6/6 on both `max,+nx,+smep,+smap` and
  `qemu64,+nx,+smep,+smap` with genuine vector 19, status `0x113`, containment,
  sibling/peer survival, slot/resource recovery, and clean shutdown.
- Registered regression: **121/121** suites passed on source commit
  `f54797f2fb5bac0b3440f986d2120382b3611ecc`.
- Official strict T2/T3/T4 stability: **10/10, 25/25, and 100/100**. The
  completed T4 run had zero failures, timeouts, or retries. A prior 93/100 run
  interrupted by executor unavailability is preserved and excluded.
- Canonical build, host suites, and 120,911-instruction kernel ISA audit
  passed. Protected Phase 14 PIE and signed APB1 identities remain unchanged.

The backend uses explicit `-accel tcg`. Its QEMU patch and results are locally
modified-emulator evidence, not upstream-QEMU or hardware-backed evidence.
The exact immutable artifacts, firmware, QEMU binaries, commands, full logs,
receipts, and fresh-extraction verifier are in
[PATCHED-TCG-QUALIFICATION.md](PATCHED-TCG-QUALIFICATION.md). The hardware-only
external kit retains its separate meaning.

No ArenaOS kernel exception-delivery workaround, synthetic interrupt, relaxed
assertion, or extended-state enablement was used. Arena 1 and Arena 2 sources
were not modified. No PR, merge, release, or Phase 16 work was created.

## Review package

The reproducible Linux package is at
[`artifacts/phase15-patched-tcg-qualification.tar.gz`](artifacts/phase15-patched-tcg-qualification.tar.gz),
with its exact SHA-256 in the adjacent `.sha256` sidecar. A fresh extraction
outside the checkout passed the independent verifier across all 2,319 file
hashes and qualification receipts. Host-side negative controls rejected missing
evidence, wrong EFI/ESP/QEMU identities, a semantic failure despite QEMU exit
zero, and failed accelerator initialization. They do not claim Windows or
hardware guest execution. Commands and outputs are in
[`receipts/package-verification`](receipts/package-verification/README.md) and
[`receipts/package-negative-controls`](receipts/package-negative-controls/README.md).

The first two packaging assembly checks exposed a hash-inventory omission and a
missing copied control-verifier helper. Both were fixed before the final archive;
the final extracted archive passed. No guest source or qualification evidence
changed as a result.
