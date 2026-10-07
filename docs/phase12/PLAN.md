# Phase 12 — Native Application Platform Foundations

**Closeout branch:** `arena/e3c48ce6-arenaos`

**Starting merged source:** `95b167198ed6bfcc1ae890697d14696daeb7fa3a`

**Qualification report:** [FINAL-REPORT.md](FINAL-REPORT.md)
**Phase 13 handoff:** [PHASE13-HANDOFF.md](PHASE13-HANDOFF.md)

Phase 12 closes the native application platform foundations present in the
merged tree. It does not attempt to deliver a complete installed-app desktop,
general runtime, or foreign ABI. Completion is tied to the exact source and EFI
receipts in `FINAL-REPORT.md`.

## Completed and qualified foundations

| Foundation | Closeout scope and evidence |
|---|---|
| Architecture and boundaries | ADR-0080 through ADR-0091 establish the native platform, APB1, startup/runtime, lifecycle, capacity, badged sessions, and protected install authority decisions. |
| Capability table and receipts | Each process has exactly 128 capability slots. The kernel and manager assert slot 127 is the last slot. The historical manager count remains the low 32 slots; slots 32–126 and slot 127 are reported separately. A Phase-9-context guest leaves slot 127 empty where no filesd install authority is issued. |
| APB1 install authority | The kernel gives the manager a filesd `BadgedEndpoint` in slot 127. After strict package readiness the manager re-describes the live cap and forwards only the exact kind/rights. It is a narrow install operation, not a filesystem root. The guest proof exercises wrong-kind and wrong-rights refusal, ordinary Filesd denial, generic LIST denial through install-only authority, restart recovery, and a real Desktop-selected signed multi-file install. |
| Desktop and IPC capacity | Queue depth 32 is exercised with a real 32-caller burst and mutation-free caller 33 refusal. Desktop holds 32 simultaneous ordinary sessions across its six built-in kinds, refuses session 33 without resource or cap mutation, closes and reuses 16 slots, and tears down to measured non-frame baselines. This is 32 one-window sessions, not multi-window-per-application support. |
| Startup and lifecycle | Native startup ABI v2 checks the exact live capability inventory. The runtime has a bounded 32-page heap, FS.base TLS setup, generation-safe integer handles, held-capability wrappers, and ProcessGroup ownership over exact Process caps. M12 guest proofs cover startup, refusal paths, app audits, and resource cleanup. |
| Package formats and AFS2 | APB1 is a canonical signed multi-file bundle with bounded files and payload. Its policy is separate from historical APKG v1 policy/records. AFS2 installation uses private staging, signature and verified-readback checks, atomic activation, and cleanup of abandoned stages. APKG v1 compatibility tests remain part of the suite. |
| Historical behavior | The affected Phase-9 through Phase-11 guest regressions, including resources, Filesd authority, application launch/lifecycle, service/client death, and window management, remain in the qualification set. |

The application registry/catalog and process-group libraries are foundations.
The closeout does not claim that the installed registry drives Desktop boot,
launcher/search, or signed installed-app execution.

## Explicitly deferred to Phase 13

Phase 13 is a separate specialist phase, **Native Runtime & Desktop Application
Maturity**. The following are not Phase-12 completion criteria and must not be
reported as implemented unless separately demonstrated:

- production boot integration for the signed installed-app registry, an All
  Applications launcher/search, and signed installed-app launch;
- broad application associations, persistent defaults, and Open With;
- true multi-window-per-application production behavior;
- complete headless/helper lifecycle and scalable process-group ownership;
- generic byte streams and stdin/stdout/stderr;
- user-created threads, process-wide mappings, and native synchronization;
- general VM reserve/commit/release/protect operations and a scalable heap;
- PIE/ASLR, if a loader implementation is still absent;
- application scaling and churn beyond the qualified 32 built-in one-window
  sessions.

A foreign syscall personality, ProcessMemory, compatd, Linux syscall semantics,
and Linux application execution are deferred to a later dedicated phase after
these native foundations mature. See [COMPATIBILITY-HANDOFF.md](COMPATIBILITY-HANDOFF.md).

## Qualification gates

1. Build and inspect the exact merged tree with the pinned repository build
   tooling, Rust host/target checks, formatting, and Python syntax checks.
2. Run targeted guest proofs for startup, 32-session capacity, slot-127
   authority, APB1 install/recovery, and affected Phase-9/10/11 lifecycle,
   filesystem, and window behavior.
3. Freeze and preserve a clean source checkpoint before running the full
   historical suite once.
4. Build the final EFI and run 100 consecutive boots from zero. Every boot must
   prove Desktop operation, two native ABI-v2 application launches and exact
   startup-cap audits, slot-127 inventory, native resource/lifecycle cleanup,
   historical boot verdicts, and clean shutdown.
5. Package `phase12-complete`, verify its hashes, extract it into a new
   directory, and boot using only its contents. Record the EFI/archive hashes,
   source checkpoint, versions, and logs in `FINAL-REPORT.md`.
