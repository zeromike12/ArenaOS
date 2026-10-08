# Phase 13 — final qualification report

## Qualification identity

- Branch: `arena/phase13-native-app-maturity`
- Exact Phase-13 parent: `330a691797343c8ef997cbb791ca54ec10e89fc5`
- Qualified Phase-12 implementation ancestor: `cd8c78a0189ce365fd5af93b0006f15fb647ed6f`
- Final OS image implementation checkpoint: `c4ad723a3e66b8b72a020589cd9ab6eab73de34d`
- Final documentation/release tip: identified in the repository branch history
  and the final completion record.
- The EFI was built from the OS implementation checkpoint above. Later commits
  changed documentation, the independent APB1 host oracle, and qualification
  packaging/witness tools; they did not change kernel or userspace OS sources.
- Final EFI SHA-256:
  `3afbceffc8c98e65790552589c5bd4bb59e4245ad4f3599d7cef093ff652f42d`
- Final ESP SHA-256:
  `147c05de9e868954c3afb929c439533e48ffa7ded2a0740740664e44865f5377`

## Qualified Phase-13 scope

The six built-in applications remain available as native applications and
regression fixtures. They no longer define the full launch inventory. The
production path now connects protected APB1 installation, trusted registry
rebuild, verified Image authority, Startup ABI v2, AppInstance/ProcessGroup
ownership, multiple windows, helpers, streams, process-wide VM, scalable heap,
user threads, and synchronization. Names and IDs select metadata; live exact
capabilities authorize operations.

### Installed registry and signed launch

At boot the trusted packaged owner enumerates protected installed candidates
through filesd. It reconstructs and verifies each candidate against the
current signer policy, the exact APB1 signed record, complete installed file
set, and payload digests. Corrupt, malformed, extra-tree, signer-revoked, or
otherwise ineligible entries are omitted. The bounded 64-entry registry is
descriptive and contains application/package identity, version, label,
executable entry, icon, associations, and multi-instance/background/headless
policy.

Desktop cannot pass a registry row, app ID, version, path, or filename to
`SYS_SPAWN`. A launch re-resolves the current installed record, reads the exact
verified executable bytes through filesd, obtains a fresh kernel-validated
Image cap from packaged, and launches that cap through the existing Startup
ABI v2 path. The dynamic Image envelope is 16 live entries of at most 256 KiB
each and 128 PT_LOAD pages; the independent dynamic child bound is 24. APKG v1
and APB1 remain separate formats and policy paths.

Host/model and receiver controls cover descriptive app IDs as non-authority,
duplicate IDs/versions, mutation-free capacity refusal, receiver/signer
denial, corrupted installed payloads and extra tree entries, source swaps,
wrong Image/rights, exact installed entry resolution, and AFS2 write-prefix
recovery. Real guest installs used signed multi-file APB1 packages and
independently compared every activated payload and `APB1.record` with the
source bundle. The archive witness repeats this install and exact AFS2 readback
from an extracted fixture.

### All Applications, dock, associations, and document authority

All Applications is built from the receiver-verified catalog. It lists
registered built-in and installed applications, supports search, keyboard and
pointer launch, reports running state, and creates a fresh verified launch.
AFS2 stores pinned favorites. The dock combines resolved favorites and active
applications; activating an existing item focuses its window instead of
duplicating its process. The mixed guest also rebooted with the same AFS2 disk
and verified favorite reload.

Associations filter installed handlers by signed content type. User defaults
are persisted as bounded, checksummed AFS2 preferences; stale IDs are pruned.
The default selects a handler but grants no authority. Desktop retains the
selected exact File cap until explicit Open/Open With selection. Cancel
creates no process and leaves process/cap counters unchanged. A selected
handler receives the exact document as a read-only File cap, with no
filesystem root; the guest checked write refusal and unchanged document bytes.

### AppInstance, ProcessGroup, helpers, and windows

Installed definition, AppInstance, ProcessGroup, process, helper, ordinary
window, and transient window have distinct records and teardown. Names, PIDs,
application IDs and window handles remain descriptive. A ProcessGroup can
hold at most four exact Process-cap members. The manager starts a helper only
from the verified package's signed allowlist and grants only the requested
Image and explicit startup capabilities. It does not copy the parent's cap
table, Desktop endpoint, document capability, filesystem roots, or network
authority.

Each ordinary window has its own surface SharedRegion, snapshot, compositor
handle, publication/damage state, close state and accounting. A single
installed native process created three ordinary windows simultaneously. The
guest closed one window while its siblings and process remained live, then
closed all windows and reaped the exact ProcessGroup. A separate signed
headless app ran without a window or Desktop endpoint and was waited/reaped
through its exact Process cap with status 42. Helper exit, terminate, crash,
stream EOF and owner-group cleanup passed. Desktop manager failure remains a
machine fail-stop because no other principal has authority to reconstruct its
process groups.

### Streams, VM, heap, threads, and synchronization

- **Streams:** Each stream set is one bounded SharedRegion page containing
  separate 768-byte single-producer/single-consumer stdin, stdout and stderr
  rings. Reads/writes support partial transfer, full-buffer `WouldBlock`,
  backpressure, explicit close, EOF, peer death, and teardown. SharedRegion
  possession authorizes bytes; explicitly granted Notification caps are wake
  hints only. The guest transferred keyboard bytes to stdin, observed stdout
  and stderr, filled a ring, resumed the producer after drain, transferred real
  parent/helper bytes, and observed EOF after close/reap. This is native
  ArenaOS behavior, not a POSIX pipe syscall.
- **VM:** Exact VmRegion caps authorize process-wide guarded reserve, lazy
  commit, RW/RO/RX protection, release and accounting. All threads use the
  same process mapping inventory. Validation requires present user PTEs and
  requested access rights; MMIO is rejected as ordinary memory; W^X is
  enforced. The ring-3 guest touched committed pages, refused guard/uncommitted
  and stale pointers, checked zeroing and read-only/executable protection,
  refused W+X, released the exact region and observed baseline accounting.
- **Heap:** `ScalableHeap` lazily reserves up to 16 MiB (4,096 pages), commits
  only needed backing, supports normal Rust allocation, reuse/coalescing,
  alignment through 2 MiB, and fallible OOM. Each operation commits at most 64
  pages; committed pages are reused after free and reclaimed at exact region
  release/process teardown. The proof allocated and reused a 256 KiB `Vec`
  with 65 committed pages, checked 64 KiB alignment and a fallible oversized
  allocation. Fixed per-process allocator metadata is 28,688 bytes. The
  measured pre-heap to active-heap sample consumed 68 free frames across the
  VM/heap proof, including page-table overhead; the heap did not commit its
  full 16 MiB reservation. Phase-12's 32-page `BoundedHeap` remains intact.
- **Threads:** Four concurrent ring-3 user workers shared the process heap and
  read-only mapping. Each used a distinct FS.base TLS block and explicit
  guarded stack, then exited and joined/detached with stable status. Quota,
  stale IDs, shared state, live-thread process teardown, helper crash cleanup,
  capability count and VM stack cleanup were exercised. User threads share
  the process capability table and acquire no new rights.
- **Synchronization:** Capability-owned SyncDomains use generation-checked
  keys and an atomic sequence-check/park operation, with wake-one/wake-all and
  bounded timeout. Native Mutex, Condvar and Once passed contention, multiple
  waiters, sequence-before-wait, timeout, invalid-cap refusal, thread death,
  and process teardown proofs. The runtime does not expose Linux futex
  semantics.

Startup ABI v2 remains exact. The final descriptor ceiling is seven, the
spawn-grant ceiling is eight including the startup transport, cap tables remain
exactly 128 slots, and slot 127 remains separately observable as APB1 install
authority. No process rights are gained from application metadata, helper
membership, window IDs, or user-thread creation.

### PIE/ASLR decision

The kernel ELF loader accepts static `ET_EXEC`, validates segment and user
bounds, enforces W^X, and rejects `PT_INTERP`; `ET_DYN` remains refused. ADR-0108
defers native static PIE/ASLR because the loader has no validated relocation
path, load-bias contract, randomized guarded placement, or Startup ABI base
propagation. No dynamic linker was added. This is the recommended next
specialist scope before Linux compatibility.

## Mixed workload and resource accounting

The representative signed APB1 pressure workload installed five ordinary
instances and one headless instance over the run; held five helpers, five
stream sets, two parked document workers, read-only File authority, timers and
notifications; and ran 21 built-in fixture instances alongside installed
applications. It reached 26 AppInstances, 32 ordinary windows, and three
windows in one AppInstance. It closed half the windows, exercised helper
failure/reap, relaunch and slot reuse, then closed all windows.

| Resource | High-water / bound |
|---|---:|
| Processes and process records | 46 / 64 |
| Scheduler threads | 49 / 64 |
| AppInstances | 26 / 32 |
| Ordinary windows | 32 / 32 |
| SharedRegions | 71 / 96 |
| SharedRegion pages | 30,555 / 36,864 |
| Shared mappings | 119 / 160 |
| Observed cap occupancy | 119 / 128; broker high-water 120 / 128 |
| IPC endpoints | 12 / 16 |
| Notifications | 56 / 64 |
| Timers | 5 at the full snapshot; 8 transient / 32 |
| VM regions | 7 at the full snapshot; 12 transient / 128 |
| VM committed pages | 355 at the full snapshot; 416 transient / 32,768 global |
| SyncDomains / keys / waiters | 5 / 2 / 2; 4 transient keys |
| Helpers / stream sets | 5 / 5; 6 transient stream sets |
| Maximum ordinary windows in one AppInstance | 3 |
| Total physical frames | 115,670 |

The mixed-workload boot identity receipt was `(records, processes,
SharedRegions, SharedRegion pages, maps, caps) = (15, 15, 2, 470, 4, 47)`.
At the 32-window snapshot, 79,188 free frames remained. After teardown,
114,110 free frames remained versus 114,189 at boot. Every measured
identity-bearing process, region, page, map, cap, thread, endpoint,
notification, timer, VM, synchronization, AppInstance, window, helper and
stream count returned to its boot value. The 79-frame residual is consistent
with the separately measured 77 retained page-table frames; exact allocator
attribution was not instrumented.

## Bugs found and RED/GREEN evidence

- The APB1 host reference parser still treated manifest bits 3 and 4 as
  reserved after the Rust receiver defined them as descriptive standard-stream
  and native-sync requests. It rejected the already-qualified Phase-13
  fixtures. The parser now accepts only defined bits 0–4, refuses bit 5 and
  above, and has positive/negative host controls. ADR-0081 now documents the
  Phase-13 extension. The APB1 reference test passed 7/7, and the real signed
  registry guest passed again after this test/oracle-only correction.
- The M10 graphics regression oracle had treated a clean QEMU exit after
  requested UEFI shutdown as an error. It now checks clean shutdown and the
  actual Desktop trusted-manager fail-stop reason. The M9 stale-pixel oracle
  had rejected the expected QEMU return code instead of gating on the actual
  fatal reason, stale pixel, and absence of retirement. Both RED controls
  remain semantic and both corrected GREEN regressions passed in the full
  suite.
- Host install/registry tests cover guessed IDs as metadata, duplicate and
  stale records, exact entry resolution, wrong signer, corrupted payloads,
  extra installed entries, source swap, receiver-policy denial and
  mutation-free refusal. Guest controls cover APB1 install authority wrong
  kind/rights, Open With cancel, exact read-only document access, invalid
  helper ID, stream-handle/Notification confusion, helper crash/reap, stale
  runtime generations and exact process-group cleanup.

## Final qualification receipts

- Historical full suite: **115/115 suite groups passed** from the frozen OS
  implementation checkpoint. The complete log is
  `build/qualification-phase13-final-suite.log`.
- After the later host APB1 reference-oracle correction, the affected format
  tests passed **7/7** and `tools/test_phase13_registry_guest.py` passed its
  full 26-AppInstance/32-window guest plus fresh favorites-reload boot
  (281.4 seconds + 6.1 seconds). No kernel or userspace OS source changed; the
  corrected affected evidence was rerun without restarting 100 boots of the
  byte-identical EFI.
- Exact EFI SHA above: `tools/stability_loop.sh 100` passed **100/100** fresh
  boots, zero failures, in 764 seconds. The runner verified that the ESP EFI
  bytes exactly matched the qualified standalone EFI before the boot set.
  Receipt: `build/stability-receipt.txt`; full log:
  `build/stability-phase13-final-100.log`.
- The repository build tooling regenerated the final ESP from the same frozen
  EFI after the 100-boot set. This final ESP has the SHA above; its bundled EFI
  was byte-compared with the stability-bound EFI and it was booted from the
  independent extracted archive witness. The 100-boot receipt binds the EFI
  hash, not FAT metadata in the ESP.
- Toolchain and firmware versions are in `TOOL-VERSIONS.txt`. The host was
  Debian 13; Rust/Cargo/rustfmt 1.97.0, QEMU 10.0.11 and OVMF 2025.02 came
  from official rustup and signed Debian snapshot packages, installed without
  root under `/tmp`.
- Archive file:
  `releases/checkpoints/phase13-complete/arenaos-phase13-complete-qemu-x86_64.tar.gz`.
  It contains the exact EFI/ESP, OVMF CODE and VARS, AFS1/AFS2 scratch
  template, two signed APB1 fixture packages, tool-version receipt,
  qualification logs, docs, ADRs, and standalone boot witness. Its extracted
  witness passed using only extracted files plus local QEMU. The
  repository also stores the captured serial, pixels, network/console logs,
  and witness output alongside the archive.
- phase13-complete archive SHA-256: `7f9fca01dcd3b3ae7e133531cf345300506a3b88248ea676daf19e9bf1ce5d2a`

The archive's included copy of this report substitutes a fixed note for the
outer archive SHA because a file cannot contain the digest of an archive that
contains that exact file. This repository copy contains the exact outer digest.

## Architecture decisions

Phase-13 decisions were recorded in ADR-0092 through ADR-0109:

- 0092 installed registry and exact launch authority; 0093 bounded native
  Image envelope; 0094 process-owned mapping metadata; 0095 native VM; 0096
  scalable heap; 0097 multiple ordinary windows; 0098 headless launch; 0099
  ProcessGroup ownership; 0100 Process-cap exit status; 0101 signed helper
  allowlist; 0102 thread yield; 0103 Desktop fail-stop; 0104 native byte
  streams; 0105 helper streams; 0106 user threads; 0107 synchronization
  domains; 0108 PIE deferral; 0109 mixed-workload resource receipt.
- ADR-0081 was updated to document the two Phase-13 APB1 request-hint bits.

## Recommended next scope

Implement and qualify native static `ET_DYN` with validated relative
relocations, randomized guarded load placement, W^X, per-launch base metadata,
and signed installed-package positive/negative fixtures. Keep dynamic linking
out of that scope. Linux/POSIX compatibility should follow only after this
native executable-loading prerequisite is resolved.
