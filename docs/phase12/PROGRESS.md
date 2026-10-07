# Phase 12 progress and receipts

**Qualification state:** source/build checks, all targeted guest proofs, the
full historical suite, the exact EFI 100/100 stability witness, and a fresh
extracted-archive boot have passed. The final report records the final repack
and repeated extracted boot: [FINAL-REPORT.md](FINAL-REPORT.md).

Phase 12 is formally **Native Application Platform Foundations**. It
qualifies the native platform foundations present in the merged tree and does
not claim a complete installed-app desktop, general runtime, or foreign ABI.
See [PLAN.md](PLAN.md) and [PHASE13-HANDOFF.md](PHASE13-HANDOFF.md).

## Qualified foundations

- Exactly 128 process-capability slots, compile-time slot-127 bound, the
  historical low-32 manager metric, and separate visibility for slots
  32–126 and slot 127.
- Queue-depth-32 IPC burst, 32 live built-in one-window Desktop sessions,
  mutation-free caller/session 33 refusal, slot reuse, and measured teardown.
- Native startup ABI v2, exact live-cap inventory, bounded 32-page heap,
  FS.base TLS, generation-safe runtime handles, and ProcessGroup ownership of
  exact Process capabilities.
- Canonical signed multi-file APB1, separate APKG v1 policy, AFS2 private
  staging/readback/atomic activation, and protected filesd/package-service
  install authority with readiness, exact-cap recheck, and restart recovery.
- Badge-authenticated built-in Desktop sessions and retained historical
  Phase-1 through Phase-11 behavior.

## Qualification completed

- All 15 host suite groups and 98 historical guest scripts passed.
- M12 startup, IPC queue, 32-session capacity, mutation-free 33rd refusal,
  reuse, and teardown proofs passed.
- M8.5/M9 resource guests observed slot 127 empty; M12/APB1 guests observed
  its exact live kind/rights and separate occupancy receipt.
- The real Desktop APB1 install proof passed wrong-kind, wrong-rights,
  ordinary-filesd, and generic-filesystem refusal checks, readback of the
  signed record and all payload files, AFS1/source preservation, and packaged
  restart recovery.
- M10/M11 launch, capability-boundary, Files, lifecycle, death, and window
  manager regressions passed.
- The exact final EFI passed 100/100 stability boots with native app startup,
  capability audits, lifecycle/resource return, Desktop input/pixels, historic
  verdicts, network/console behavior, and clean shutdown.

The environment restarted during the full suite. The remaining tests were
continued at the same source SHA; the two tests interrupted inside deliberate
source mutations were restored and rerun through their cleanup paths. Exact
counts, resource high-water values, hashes, logs, and the archive result are
recorded in [FINAL-REPORT.md](FINAL-REPORT.md).

## Deferred work

The installed-app registry launcher, associations/Open With, multi-window
applications, headless/helper lifecycle, streams, user threads/synchronization,
process-wide mappings, general VM, scalable heap, and broader churn belong to
Phase 13. Foreign ABI, ProcessMemory, compatd, Linux syscall semantics, and
Linux application execution belong to a later dedicated phase and are not
implemented here. See [COMPATIBILITY-HANDOFF.md](COMPATIBILITY-HANDOFF.md).
