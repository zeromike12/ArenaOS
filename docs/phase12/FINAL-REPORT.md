# Phase 12 — Native Application Platform Foundations

## Qualification status

Phase 12 is qualified under the revised scope. The merged implementation
passed the source, build, host, guest, regression, and 100-boot stability
gates. A fresh extraction of `phase12-complete` also passed its checksum
manifest, representative native/APB1 guest behavior, and clean shutdown.

## Source and artifact identity

- Repository branch: `arena/e3c48ce6-arenaos`
- Starting merge SHA: `95b167198ed6bfcc1ae890697d14696daeb7fa3a`
- Qualified implementation checkpoint: `cd8c78a0189ce365fd5af93b0006f15fb647ed6f`
- Starting tree was clean. `git diff --check` passed and no unresolved merge
  markers were present.
- The implementation checkpoint is pushed. The final report/handoff update is
  documentation-only and follows the tested implementation checkpoint.
- Final EFI: 6,360,576 bytes,
  SHA-256 `963053de20b32dbc354c20aecfdb9f1672a42f22b6bf46a2b63e478d651b1faa`
- Final 8 MiB ESP: SHA-256
  `26c7a5414c7b0e22d7a303e13a957036a9eeef406cad1206158d051035df274a`
- The EFI extracted from the ESP was byte-identical to the independently
  hashed EFI. The 100-boot witness guarded this EFI hash throughout.
- The qualified code source remained clean at the implementation checkpoint.

## Environment and build qualification

The host was Debian GNU/Linux 13 (trixie), Python 3.12.14. Rust was installed
through official rustup; the repository has no root-level toolchain pin. QEMU
10.0.11 and OVMF 2025.02-8 were obtained from the trusted Debian repository and
used from an unprivileged local extraction. Exact tool versions, targets,
firmware paths, sizes, and hashes are in [TOOL-VERSIONS.txt](TOOL-VERSIONS.txt).

`tools/dev-env/env.sh` and the repository build flow were used. The final
`tools/build.sh --image` build passed, including the kernel's no-FPU/SSE/MMX
image audit. Rust formatting, host tests, `-D warnings` clippy checks, pinned
crate release builds for `x86_64-unknown-none`, and the UEFI workspace check
passed. The corrected target-check transcript is
`build/phase12-stage-a-corrected.log` in the qualification workspace.

An early ad-hoc target-build command was launched from the repository root and
therefore missed the crate-local `.cargo/config.toml`: the platform build used
the wrong curve25519 backend, and the no-std startup proof was linked as a
Linux binary. Those invocations failed; no source workaround was made. The
same checks were rerun from each crate directory under its pinned target and
serial-backend configuration and passed. The canonical full-suite commands
also ran in the crate directories and passed.

## Historical and targeted suite results

The full historical suite covered all 15 host-suite groups and every one of
the 98 `tools/test_m*.py` guest scripts: **113/113 passed, zero failures**.
Coverage includes historical Phase-1 through Phase-11 behavior, APKG v1,
AFS2, M8.5/M9/M10/M11 resources and lifecycle, and the Phase-12 startup,
capacity, and install proofs. The complete transcript is
`build/qualification-full-suite.log` in the qualification workspace and is
included in the archive.

The managed environment restarted during three points in the suite. The
unrun suffix was continued at the same source SHA. It stopped twice while
red-control tests temporarily edited source: `test_m85_manager_destroy.py` and
`test_m9_shared_unmap_red.py`. Their exact intended mutations were restored,
each test was rerun from a clean tree through its restoration and guest-green
checks, and both passed. No failed test was accepted as flaky or omitted.

Phase-12-specific guest results:

- `test_m12_startup.py`: ABI-v2 startup and exact capability audit, argv and
  environment, FS.base TLS across a scheduler handoff, 32-page heap boundary,
  generation-safe handles, exact Process-cap spawn/wait/reap, five startup
  refusal controls, and exact process teardown all passed.
- The same M12 guest run accepted 32 blocked IPC callers, refused caller 33
  with `STATUS_BUSY`, served every accepted call exactly once, and restored
  threads and endpoints.
- `test_m12_scale.py`: 32 simultaneously live ordinary Desktop sessions
  across all six built-in application kinds, 32 unique clocks, 64
  session-owned regions, and 30,550 measured pages passed. Session 33 was
  refused with exact capability and resource inventories unchanged. Sixteen
  sessions closed and their slots were reused; all 32 then retired.
- `test_m85_resources.py`: guest-measured process records/processes rose from
  16/16 to 17/17 for a live child and returned to 16/16 after exact
  Process-cap retirement. Manager capability occupancy and the bounded
  64-notification budget were observed; notification 65 was refused and
  temporary probe slots were reclaimed.
- `test_m9_resources.py`: the original child returned the guest from 16/16 to
  15/15 records/processes; SharedRegion references fell 3/80 to 2/80, pages
  507 to 488, maps 6 to 4, and compositor capabilities 10/128 to 7–8/128.
- M10 app, dynamic-launch, boundaries, client/service death, and Files tests;
  M11 window-manager and Files tests all passed. Their guest runs exercise
  launch, held Process-cap lifecycle, badge boundaries, file authority,
  teardown, and window management.

## Slot 127 and APB1 authority proof

The process table is exactly 128 slots, with compile-time checks that slot 127
is the final valid slot. The historical manager inventory remains the low-32
count; `[32,127)` and slot 127 are separately observable. M8.5/M9 resource
contexts report slot 127 empty with descriptor/kind/rights all zero. The
APB1 context separately observes the live slot-127 capability as kind 12,
rights 6, while `[32,127)=0` and the historical low-32 count remains unchanged.
The slot number carries no authority: the manager describes and validates the
exact held live capability before forwarding it.

`test_phase12_apb1_guest.py` passed on a real guest. It selected the signed
multi-file fixture through the Desktop, installed it into AFS2, verified the
signed record and all three payload files by readback, preserved the Desktop
source and AFS1 region, and verified cleanup of the private staging tree. It
also proved that ordinary filesd authority cannot invoke protected install,
install-only authority cannot perform generic `LIST`, and wrong-kind and
wrong-rights capabilities are refused. After packaged-service restart the
manager re-established and forwarded the handoff again.

The guest receipt ordering was strict package `READY`, then exact held-cap
occupancy/re-description, then handoff. The 15-test manager host suite includes
`coalesced_apb1_wake_is_preserved_but_cannot_complete_readiness`: the APB1 bit
is retained as sideband, removed from the readiness mask, and cannot satisfy
the independent required-driver gate. The production manager uses that path;
the guest proof then exercises strict readiness, exact live-cap validation,
forwarding, refusals, and restart recovery.

## Resource high-water and cleanup

The M12 scale tuple is ordered as free frames, process records, live processes,
SharedRegion records, mapped pages, maps, and occupied caps:

| Snapshot | Guest-measured tuple |
|---|---|
| Baseline | `(115372, 15, 15, 2, 470, 4, 45)` |
| 32 sessions live | `(79425, 47, 47, 66, 30550, 110, 109)` |
| Half closed | `(97360, 31, 31, 34, 15510, 58, 77)` |
| 32 reused sessions | `(79424, 47, 47, 66, 30550, 112, 109)` |
| Final teardown | `(115295, 15, 15, 2, 470, 4, 45)` |

The final state returned all identity-bearing resource counts to baseline.
The 77-frame difference is retained empty page-table frames, explicitly
measured by the guest. The M8.5/M9 slot-127-empty checks and M12/APB1
slot-127-held checks independently verify that the reserved authority is
visible where issued and absent in the historical contexts.

## 100-boot stability witness

`tools/stability_loop.sh 100` passed **100/100** from zero in 748 seconds on
the exact final EFI above. One failed boot would have invalidated the attempt;
none failed. Every boot used fresh UEFI variables and a fresh AFS2-capable
scratch disk. Each run checked a live Desktop, two native ABI-v2 launches and
capability audits, QMP pixels/input/drag/relaunch and `LIFECYCLE 2 2`, slot-127
kind/rights and high-slot inventory, complete application resource return,
historical suite verdict consistency, network/console fixtures, and clean
shutdown. The exact receipt is `build/stability-receipt.txt`; all 100 bounded
desktop receipts are included in the archive.

## Qualified Phase-12 scope

Phase 12 is **Native Application Platform Foundations**. Qualified scope is:

- the application/platform architecture and ADR boundaries;
- exactly 128 process-capability slots, separate slot-127 accounting, and
  unchanged low-32 manager inventory;
- queue-depth-32 burst handling and 32 simultaneous built-in one-window
  Desktop sessions with mutation-free 33rd refusal and teardown;
- lifecycle/application-instance foundations, `ProcessGroup` ownership over
  exact Process capabilities, and generation-safe runtime handles;
- canonical signed multi-file APB1, policy separation from APKG v1, and the
  AFS2 private-stage, verified-readback, atomic-activation design;
- protected filesd/package-service APB1 install authority and restart recovery;
- native startup ABI v2, exact startup-capability inventory, the runtime
  startup gate, bounded heap, and FS.base TLS;
- application catalog/registry and lifecycle APIs as foundations; the registry
  is not wired into Desktop boot, the launcher, or signed installed-app launch;
- badge-authenticated built-in Desktop sessions and historical Phase-1 through
  Phase-11 behavior covered by the full suite.

Endpoint object numbers, PIDs, and numeric slot values are descriptive. Exact
live capabilities, their kinds/rights, and generations determine validity and
authority. No endpoint-ID-zero invalidity rule is assumed.

## Explicitly deferred

Phase 13 is **Native Runtime & Desktop Application Maturity**. Its handoff
lists exact current APIs, ADRs, and source files and covers production registry
boot integration, launcher/search, signed installed-app launch, associations
and Open With, true multi-window apps, headless/helper lifecycle, generic
streams, user threads and synchronization, process-wide mappings, general VM,
a larger heap, and broader app scalability/churn. See
[PHASE13-HANDOFF.md](PHASE13-HANDOFF.md).

Foreign syscall personality, ProcessMemory, compatd, Linux syscall semantics,
and Linux application compatibility are not implemented or implied; they are
reserved for a later dedicated compatibility phase. See
[COMPATIBILITY-HANDOFF.md](COMPATIBILITY-HANDOFF.md).

## Archive closeout

The final `phase12-complete` archive contains the exact EFI/ESP, OVMF firmware,
AFS2 disk template and signed fixture, boot tools, ADRs, closeout documents,
qualification logs, all 100 desktop receipts, and a SHA-256 manifest. The
manifest validated every extracted file. The fresh extraction was located at
`/tmp/arenaos-phase12-extract.xNyDCB/phase12-complete`; its boot used only
files from that directory and the installed QEMU executable. The extracted
guest matched the final EFI SHA above, launched and retired two native apps,
installed APB1 v42 through the Desktop, read back the signed record and all
three payload files from AFS2, preserved source and AFS1, exercised network
and console fixtures, and shut down cleanly. The boot receipt is
`build/phase12-extracted-boot.log` in the qualification workspace.

The archive SHA-256 is supplied in the detached `.sha256` companion. The
companion is outside the archive because an archive cannot contain its own
digest. The archive is built after this final report and its extracted boot is
repeated against that exact final package.
