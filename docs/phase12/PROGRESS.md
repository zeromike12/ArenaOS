# Phase 12 progress and receipts

**Current closeout status:** targeted source, host, and guest checks are green;
the exact full historical suite, artifact-bound 100/100 loop, and extracted
`phase12-complete` archive boot remain closeout gates. The authoritative final
status will be recorded in [FINAL-REPORT.md](FINAL-REPORT.md).

The earlier Phase-12 prompt asked for a much broader runtime and foreign-ABI
wave. The agreed closeout scope is now **Phase 12 — Native Application
Platform Foundations**. Completed native foundations are qualified without
claiming an installed-app desktop or Linux compatibility. See [PLAN.md](PLAN.md)
and [PHASE13-HANDOFF.md](PHASE13-HANDOFF.md).

## Implemented foundations

- Architecture and bounded-capacity records: ADR-0080 through ADR-0091.
- Exactly 128 process capability slots; the manager keeps its historical
  low-32 metric, separately counts slots 32–126, and reports slot 127's exact
  descriptor. Slot 127 is held `BadgedEndpoint` authority for APB1 install,
  not authority by numeric slot value.
- Queue depth 32 and 32 live built-in Desktop sessions, with mutation-free
  session-33 refusal, half teardown, slot reuse, and measured resource return.
  Each session still represents one process and one window.
- Native startup ABI v2, exact live-cap inventory, bounded 32-page heap,
  FS.base TLS setup, generation-safe runtime handles, and exact Process-cap
  group lifecycle.
- Canonical signed multi-file APB1; policy remains separate from APKG v1.
  Protected filesd installation verifies signature and durable readback before
  atomic AFS2 activation. The manager performs strict readiness, exact cap
  recheck, slot-127 handoff, and restart recovery.
- Historical APKG v1 and Phase-11 package, filesystem, Desktop, and lifecycle
  behavior remain in the regression suite.

## Targeted qualification completed

- `tools/test_m12_startup.py` and `tools/test_m12_scale.py`: startup/heap/TLS/
  handle/ProcessGroup gates, queue 32/33 boundary, 32 real sessions, refusal,
  reuse, and teardown.
- `tools/test_m85_resources.py` and `tools/test_m9_resources.py`: historical
  resource high-water, capability occupancy, refusal, and exact cleanup.
- `tools/test_m10_apps.py`, `test_m10_dynamic.py`, `test_m10_boundaries.py`,
  `test_m10_client_death.py`, `test_m10_service_death.py`, and
  `test_m10_files.py`: native launch, signed dynamic launch/revocation,
  capability boundaries, lifecycle failure, and Files authority.
- `tools/test_m11_wm.py` and `tools/test_m11_files.py`: window manager and
  file-capability behavior.
- `tools/test_phase12_apb1_guest.py`: real Desktop-selected signed APB1 install,
  protected AFS2 readback, wrong-kind/wrong-rights refusal, generic-filesystem
  denial, ordinary Filesd denial, package restart recovery, slot-127 receipt,
  and clean shutdown.
- `servicemgr::readiness` unit proof: coalesced APB1/readiness wake preserves
  the sideband but cannot complete strict readiness; the manager re-describes
  the live authority before handoff.

Run commands, exact results, high-water values, hashes, and outstanding
release gates are recorded in [FINAL-REPORT.md](FINAL-REPORT.md).

## Deferred to Phase 13 and later

Production installed-app registry boot/launcher/search and signed installed-app
launch, Open With and associations, multi-window-per-app, headless/helper
lifecycle, streams, user threads/synchronization, general VM, scalable heap,
and broader native app churn belong to Phase 13. Foreign ABI syscall proxy,
ProcessMemory, compatd, Linux semantics, and Linux app execution are later work.
They are not Phase-12 completion claims; see
[COMPATIBILITY-HANDOFF.md](COMPATIBILITY-HANDOFF.md).
