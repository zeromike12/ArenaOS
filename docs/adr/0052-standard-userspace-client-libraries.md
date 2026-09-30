# ADR-0052 — Phase 8.3 reusable no_std client libraries

*Status: Accepted and implemented; Phase 8.3 qualified (2026-09-30). This is an internal userspace factoring decision, not a new kernel primitive, public syscall ABI, capability type or persistent format.*

## Problem

`userspace/abi.rs` is text-included into every binary; `userspace/net.rs` is included only in `arptest`, and filesystem callers in `fstest`, `permissiond`, `configd` and the shell hand-assemble 64-byte IPC messages and duplicate the transport/returned-cap checks. The M7.7 native networking module is already a good bounded client API but a module exercised only in its original test image is not a standard userspace library. In particular, its syscall transport currently discards a returned capability word instead of refusing one. A general-purpose library must be compiled and linked by **independent real guest images**, not only included by the test from which it came; host mocks alone cannot demonstrate held-cap authority or compatibility with fsd/netstackd.

## Decision and authority boundary

Add one separately compiled `no_std` rlib (`userspace/arena-lib`) with the following narrowly bounded modules. Use Cargo path dependencies from independent existing standalone userspace crates; do not create another image, kernel ABI, IPC primitive, allocator, threading facility, POSIX/socket facade or ambient cap registry.

* `abi` reuses the existing frozen `userspace/abi.rs` declarations unchanged; it does not own or mint a cap. `sys` exposes only typed wrappers around existing syscall numbers; `ipc` implements a synchronous, caller-owned 64-byte message/three-word reply exchange, requires a *passed* endpoint slot, and rejects unexpected returned caps with a typed protocol error. A returned cap cannot silently become authority merely because a generic client ignored it. Transport errors and service statuses remain distinct; no wrapper converts SERVICE_GONE into success or retries a potentially non-idempotent operation.
* `fs` owns the FS wire packing and a bounded client using `ipc`. Typed name/length, LS cursor, OPEN/CREATE, READ/WRITE with an explicitly supplied LENT cap, CLOSE and UNLINK helpers check message/length/returned-cap contracts. Low-level `request` remains available for the existing receiver-gated fsd shutdown diagnostic and exact historical syscall-order fixture. A client **does not own a file by name**: it holds an explicit endpoint slot and an opaque fsd handle, and cannot read data without the caller's separately held LENT buffer cap. The library never copies an fsd endpoint into an app and never raises rights.
* `net` is the existing M7.7 `userspace/net.rs` module compiled in the rlib, with its production syscall path using the checked `ipc` boundary. It keeps explicit bearer possession, `adopt_*` delegation and CLOSE; no pid/identity test or UDP-specific kernel cap. The existing host-injected `Transport` contract remains supported.

**Two genuinely independent guest consumers per layer:** the historical `fstest` and the resident `permissiond` call the linked `fs` client (the latter uses typed operations on the durable-policy path); `arptest` and the shell call the linked `net` client (the shell's opt-in library command performs a real live-guest slirp ARP lookup on its already-held stack endpoint). The `fs` and `net` clients both traverse the same checked `ipc`/`sys` rlib path, so their independent guest usages also exercise those layers. Keep all existing historical wire tests and exact service operation counts: refactoring the client must not mutate protocol requests or introduce extra operations. The trusted shell keeps its existing Power/raw-FS privileges; the app and broker still inherit exactly the ADR-0048 one/four grants. A library object is not a grant.

## Proof and rollback rules

Host-build/test the rlib for native and `x86_64-unknown-none`, with a mock transport proving exact byte/word layouts, missing/wrong-kind/overlong input refusal, cap-return rejection, transport-vs-status separation, and no unintended retry. Keep the M7.7 real client host contracts. Real QEMU guests must prove two independent FS consumers against the same actual fsd and two independent net consumers against real wire; no fabricated reply or fake peer. Existing M5 persistent disk-operation counts, permission durability/crash/refusal cases, network bearer/restart/negative-space checks, cap inventory and fixed resource assertions must stay green. New `libnet` shell proof is opt-in and tests real endpoint/wire with device-absent SKIP, not an automatic boot mutation. Preserve the entire historical suite and qualify any completed 8.3 checkpoint with a **fresh final-EFI-bound 100/100**, matching archived receipt, checksums and boot of the extracted deployable image. An internal partial pass is not a milestone checkpoint and is not committed without its deployable qualified image. If a foundational refactor changes privilege grants, the syscall/public ABI, or the AFS1 persistence contract, stop and make a separate decision; this ADR does not authorize those changes.

## Qualified implementation and failure analysis (2026-09-30)

The independent consumers link a separately compiled `arena-lib` rlib for
`x86_64-unknown-none`: `fstest` and resident `permissiond` use the checked
FS client; `arptest` and the shell use the native network client. The shell's
opt-in `netlib` path proves real gateway ARP on its held stack cap, with a
kernel-refused wrong-kind Image cap and an honest no-device SKIP. The FS
fixture retained exactly **34** disk operations and the broker's durable
ALLOW/READ/REVOKE survived unchanged. Host rlib tests **2/2**, native
network fake-transport tests **5/5**. Final historical gate:
`build/phase83-qualified-full-suite.log`, **42/42**; focused linked-guest
script `tools/test_m83_libraries.py` PASS. A **fresh final-image**
`tools/stability_loop.sh 100` was **100/100**, no failures, with receipt
`e17798ce1e97cef9ddbf670e07df5b78bafbbe5c80114bfb2f3b4cfcfa387f2b 100/100`
for the exact EFI within the shipped ESP. The deployable archive
`releases/checkpoints/phase83-complete/arenaos-phase83-complete-qemu-x86_64.tar.gz`
has SHA-256 `9b0390891b59febbad1d7aba548f4d1d7d5ce1188b20a7f5af2e41c3c3634e4b`;
all extracted member checksums, bundled EFI/ESP identity, receipt, bundled
firmware and formatted AFS1 disk were verified. An **independent extracted
archive boot** completed the native `netlib` real-wire proof, all embedded
milestone passes and a clean Power-cap shutdown; the disk audited clean.
Boot instructions are in `docs/RUNNING.md` and bundled `RUNNING.md`.
This archive is a per-commit checkpoint, not a tagged GitHub release.

The first full integration attempts failed, and were not used as evidence:
resource-baseline fixtures sent commands at the first shell prompt while a
concurrently spawned temporary permission app still held one Process and
16 frames. The actual manager Process-cap reap occurred *after* the initial
12/12 snapshot but before the later 11/11 snapshot. This was neither a
frame leak nor permission authority to discard. All affected host feeders
now wait for the actual reap **and** the relevant production readiness and
prompt before exact snapshot/restart probes. No numeric resource assertion
was weakened. A shell echo can appear adjacent to its prompt or be
interleaved with service logs; `stackstop` now accepts either form without
accepting an unrelated `m8:` diagnostic as the typed command. One historic
M7 native DNS continuation call failed after a previous raw UDP query had
used the identical source/destination/TXID; this alone does not identify
whether the DNS proxy coalesced, dropped, or delayed the response. The
second query now uses a **distinct TXID** so a matching reply proves a
separate transaction; failures now log typed transport/service/protocol
status. A single earlier boot stalled before kernel output with only
87 OVMF serial bytes; QMP remained running but its CPU diagnostic had an
incorrect `command-line` key. That key is corrected (subsequent timeout
captured actual guest RIP/HLT at a shell awaiting an incorrectly spelled
test marker). The pre-kernel stall did **not** recur in the final suite or
100/100; its cause is not established and no flake is counted as a pass.
The old Phase 8.2 checkpoint at `9350a48` remains unchanged.

## Rejected alternatives

* Merely moving `net.rs` into a new path while only `arptest` includes it: one test client is not a reusable library.
* Treating a name/pid/caller identity, an implicit globally configured endpoint, or a `Drop` handler as authority/revocation. Possessed caps and receiver-issued bearers keep their original semantics.
* Unchecked generic IPC that silently drops unexpected cap transfers, or a filesystem convenience API that grants raw fsd to the Phase 8.2 app.
* Bundling the whole kernel/userspace ABI into a new dynamic runtime or adding a new service image to make a library look exercised. This milestone is a linked, no_std client factoring with at least two existing independent users, not 8.4 packages or 8.5 installation.
