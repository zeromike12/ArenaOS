# Phase-14 handoff — native application platform

Phase 13 is complete on the qualified implementation checkpoint recorded in
`FINAL-REPORT.md`. The recommended next specialist scope is **native executable
loading: static PIE and ASLR**. Do not begin Linux compatibility as the next
automatic step: ArenaOS still accepts only static `ET_EXEC`, and the current
ELF loader has no relocation or randomized placement contract.

## Native platform available

### Installed packages and launch

- APB1 v1 is a signed multi-file application format distinct from APKG v1.
  Receiver-side signer policy and filesd verification remain authoritative.
  AFS2 stores immutable installed records and payloads below the protected
  `/System/Applications/<app-id>/<version>` namespace.
- At boot the trusted packaged owner enumerates installed candidates, verifies
  current signer policy, signatures, the signed record, exact file set and
  payload digests, then publishes a bounded 64-entry descriptive catalog.
  Desktop receives metadata only. A launch re-resolves the current installed
  record and creates a fresh kernel-validated Image capability from the exact
  verified executable bytes.
- APB1 manifest identity, package ID, version, label, icon, path, file type,
  requested startup profile, and application IDs are selectors and metadata.
  None grants launch, filesystem access, process control, or service access.
  APKG v1 policy and its installer remain separate.
- Dynamic native Images are bounded at 16 live entries, 256 KiB per Image,
  and 128 PT_LOAD pages per Image. The separate dynamic-child budget is 24.
  The loader accepts strict static `ET_EXEC` only, retains W^X and user-range
  checks, and refuses `PT_INTERP` and `ET_DYN`.

### Application, process, and window lifecycle

- An installed application definition, an `AppInstance`, its `ProcessGroup`,
  each process/helper, and each window have separate records and teardown.
  IDs and names locate state; exact live capabilities authorize operations.
- A ProcessGroup has at most four explicitly granted Process members. A helper
  is resolved by a signed allowlist record and a verified payload entry. The
  manager passes only the requested exact Image and explicit startup grants;
  it does not clone the parent's cap table, document authority, Desktop
  endpoint, filesystem roots, or network capability.
- One AppInstance can own no windows, one, or multiple ordinary windows. The
  total Desktop policy remains bounded at 32 windows. Each window has a
  separately backed SharedRegion and compositor handle, its own publication,
  damage, state and close lifecycle. The guest created three windows in one
  process and closed them independently.
- A headless installed application receives no window or Desktop session
  endpoint. The owner waits and reaps through its exact Process cap, observes
  stable exit status, and retires the instance resources.
- Desktop and packaged fail-stop if their trusted lifecycle authority dies;
  no PID, path or descriptive registry entry can reconstruct that authority.

### Document access and application catalog

- All Applications queries the verified installed catalog, filters by search,
  supports keyboard and pointer selection, displays current-running state, and
  launches through fresh verified Image authority. The dock is derived from
  AFS2-persisted favorites and active applications.
- APB1 content associations and user defaults are preferences only. Open With
  holds the selected exact File capability until a user selects a handler.
  Cancel consumes the pending offer without launch. On explicit launch,
  filesd supplies an exact read-only File cap; it does not hand the app an
  ambient filesystem root. Stale handler IDs are pruned when the registry is
  rebuilt.

### Startup, streams, memory, and threads

- Native Startup ABI v2 remains the launch contract. Descriptors are an exact
  bounded inventory: at most seven descriptors and eight spawn grants
  including the startup transport. Startup role references identify exact
  held capabilities; role names and indices do not grant authority. The
  process cap table remains exactly 128 slots and reserved slot 127 remains
  separately observable.
- Native stdin/stdout/stderr and helper channels use one bounded page-backed
  SharedRegion per stream set, containing three 768-byte single-producer /
  single-consumer rings. Reads/writes support partial transfer, backpressure,
  EOF, peer closure, and wake notifications. The SharedRegion cap authorizes
  bytes; notification caps are wake hints only. This is not a POSIX pipe ABI.
- Process mappings are process-owned and shared by all of the process's
  threads. Exact VmRegion caps support guarded reserve, lazy commit, RW/RO/RX
  protection, W^X refusal, release and accounting. Regions are bounded at
  eight per process, 128 globally, and 4,096 pages per region; commits are at
  most 64 pages per operation and process/global committed limits are
  8,192/32,768 pages. User pointer validation requires the current present
  process PTE and requested access permission. VM and teardown reclaim owned
  mappings; MMIO cannot appear as ordinary RAM.
- `ScalableHeap` lazily reserves 16 MiB of virtual capacity; it commits only
  the pages required by Rust allocations and reports OOM through fallible
  allocation. Freed blocks coalesce or reuse committed runs; exact VM release
  or process teardown returns backing. The Phase-12 32-page `BoundedHeap`
  remains available. No capability slot is borrowed as a heap frame holder.
- Each process can create at most four additional ring-3 threads within the
  existing 64-slot global scheduler. Threads share that process's PML4 and
  exact cap table; they receive no new rights. Each thread has a guarded
  explicit stack, unique FS.base TLS and stable same-process join/detach
  lifecycle. Process teardown kills live threads and releases their stacks.
- The native synchronization runtime uses a capability-owned SyncDomain,
  generation-checked keys, atomic sequence-and-park wait, wake-one/wake-all,
  a bounded timer wait, and process-owned key cleanup. Mutex, Condvar and Once
  are built over this ArenaOS interface. It does not import Linux futex or
  POSIX synchronization semantics.

## Qualified mixed workload

The signed guest reached 26 live AppInstances, 32 ordinary windows, a
three-window AppInstance, five installed applications, five live helpers,
five stream sets, two parked document workers, active timers/notifications,
and a separate headless launch. It closed half the windows, exercised helper
failure/reap, reused freed slots, then closed all windows and all process
groups. Identity-bearing resource counters returned to their baseline.
The final 79-frame free-memory delta is recorded alongside the known 77
retained page-table frames; allocator attribution was not isolated.

## Recommended Phase 14: native static PIE/ASLR

Start from ADR-0108. Add a narrow, statically linked `ET_DYN` subset with
validated `R_X86_64_RELATIVE` relocations, randomized load bias, guarded
placement gaps, and the actual base in Image metadata and Startup ABI v2.
Keep the exact Image capability and package-policy trust chain; test positive
and malformed signed APB1 packages through real install and launch. Prove
W^X, no overlap with stack/VM reservations, independent per-launch placement,
stale/revoked Image cleanup, and source-policy preservation. No dynamic linker
is required for that scope.

Once native executable placement is qualified, a compatibility phase can
decide how its executable and syscall model maps to ArenaOS. Native threads,
streams, mappings, synchronization, process ownership and teardown are now
concrete substrate mechanisms; they do not prescribe Linux/POSIX semantics.
