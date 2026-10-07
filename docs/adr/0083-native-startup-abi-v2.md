# ADR-0083 — Native process startup ABI v2

**Status: Accepted and guest-qualified for the built-in ABI-v2 startup path; installed-app registry launch, general VM, and user-thread lifecycle remain open.**

**Date:** 2026-10-06. **Milestone:** Phase 12, native application launch. **Revision:** the descriptive instance-slot domain is 0..31, aligned with the accepted 32-entry manager/session envelope in ADR-0088.

**Related decisions:** ADR-0002, ADR-0015, ADR-0019, ADR-0056, ADR-0080, ADR-0084, ADR-0085.

## Problem

The current native spawn path enters an ELF image at its validated entry point
with one derived 4 KiB stack page. It does not define `argc`/`argv`, environment,
CWD, standard streams, or a versioned startup record. New applications need a
stable launch contract without changing the legacy Phase-11 entry path or
putting application names and paths into kernel authority.

## Decision

### Additive, userspace-owned transport

- ABI v2 is selected by the trusted userspace application manager. The manager
  explicitly inherits one newly created, one-page `SharedRegion` as child cap
  slot 0. It fills the page before spawn and gives the child only `READ|DESTROY`
  rights; the child maps it read-only, parses/copies or borrows its contents,
  then unmaps and destroys the startup cap when it no longer needs it. The
  manager's creation cap is not implicit child authority.
- Slot 0 is a startup-data transport, not a path, Image, Process, CWD, or file
  capability. A runtime must verify the actual held slot with
  `SYS_CAP_DESCRIBE` (kind 7, SharedRegion), verify its page count is exactly
  one, and map it with `writable = 0`. Missing, malformed, stale, or mismatched
  startup data fails closed; there is no fallback to a guessed ABI.
- The current kernel spawn limit is five inherited caps. Slot 0 is reserved
  for the startup page, leaving at most four additional explicit cap
  descriptors in this ABI. No kernel cap/process/thread/page limit is raised.
  The child cap table remains the authority; the block's slot numbers and
  type/rights descriptions are checked against `SYS_CAP_DESCRIBE` and do not
  mint or amplify rights.
- Existing ABI-v1/legacy `SYS_SPAWN` users are unchanged. The kernel does not
  parse application arguments, manifests, CWD strings, or this record.

### Entry contract

- The ELF entry is the validated executable entry address. `RSP` points at the
  exclusive top of the one-page user stack, is 16-byte aligned, and has no
  synthetic return address or hidden startup block on it. The initial user
  flags have IF set and DF clear. All general-purpose and vector registers are
  unspecified input and must be ignored by the ABI-v2 entry stub; no register
  carries authority or process arguments. This documents the existing entry
  shape rather than changing the Phase-11 register state.
- Runtime entry code must not allocate before validating the startup page. The
  page is the complete bounded source for arguments, environment and initial
  cap descriptors; all authority stays in the exact child cap slots. The
  reusable `arena-runtime` gate copies the mapped page into fixed private BSS,
  unmaps and destroys slot 0, parses the immutable copy, compares every listed
  live cap kind/rights, and only then invokes application code. Its one-shot
  scratch is not a heap allocation or a large initial-stack object.
- Page size is 4,096 bytes. The record is little-endian and versioned, with a
  fixed 128-byte header, bounded descriptor tables and one contiguous string
  pool. Canonical offsets are contiguous and exact; unused bytes through the
  end of the page are zero. `argc` is 1..=32 (argv[0] nonempty), `envc` is
  0..=32 (each entry nonempty), and at most four additional capability
  descriptors are present. The combined argv/environment byte pool is at
  most 3,072 bytes; strings are non-NUL byte sequences, not path authority.
  No inherited environment is implicit.
- Header fields are frozen as follows:

  | Offset | Bytes | Field |
  |---:|---:|---|
  | 0 | 4 | `ARST` magic |
  | 4 | 2 | version = 2 |
  | 6 | 2 | header bytes = 128 |
  | 8 | 4 | exact total bytes, including tables/string pool |
  | 12 | 4 | descriptive manifest flags; only multi-instance/background/headless |
  | 16 | 32 | canonical NUL-padded application ID |
  | 48 | 2 | userspace instance slot (0..31) |
  | 50 | 2 | zero reserved |
  | 52 | 8 | nonzero instance generation |
  | 60 | 2 | argc |
  | 62 | 2 | envc |
  | 64 | 2 | additional-capability count (0..4) |
  | 66 | 2 | zero reserved |
  | 68 | 4 | argv descriptor offset |
  | 72 | 4 | environment descriptor offset |
  | 76 | 4 | capability descriptor offset (always 128) |
  | 80 | 4 | string-pool offset |
  | 84 | 4 | string-pool bytes |
  | 88 | 2 | CWD descriptor index or `0xffff` |
  | 90 | 2 | stdin descriptor index or `0xffff` |
  | 92 | 2 | stdout descriptor index or `0xffff` |
  | 94 | 2 | stderr descriptor index or `0xffff` |
  | 96 | 4 | page size = 4,096 |
  | 100 | 4 | zero reserved |
  | 104 | 8 | validated executable entry address |
  | 112 | 8 | page-aligned executable load base |
  | 120 | 8 | initial monotonic microseconds (data only) |

  `instance_slot` uses one of 32 bounded userspace slots, not the old
  16-entry host-model placeholder. The bound matches the 32-entry
  `AppInstanceTable` model and the Desktop's 32 live session records; the
  32-window table remains a separate capacity, and one app instance may own
  multiple windows. This built-in integration currently uses the Desktop
  session row as the slot; wiring the reusable lifecycle table to the exact
  Process-cap records remains a separate gate. A live slot is not folded onto
  another live slot: the single-threaded manager chooses a free index before
  creating resources and publishes it only after successful spawn. The
  current built-in launcher writes that exact index and a nonzero monotonic
  per-launch generation. Both are descriptive and cannot select an IPC
  session; ABI-v2 service dispatch authenticates the kernel-delivered endpoint
  badge (ADR-0090). Reuse changes the generation.

  Tables follow contiguously in this order: `cap_count` 16-byte capability
  descriptors, `argc` 8-byte argv descriptors, `envc` 8-byte environment
  descriptors, then the exact string pool. Each string descriptor is
  `(pool-relative u32 offset, u32 length)`; descriptors cover the pool
  contiguously in argv-then-environment order with no gaps or aliases.
  Capability descriptors are `(child slot u16, role u8, describe-kind u8,
  exact rights u32, zero reserved u64)`. Child slots are exactly 1..=count;
  slot 0 is the startup region and is not repeated in the table. Roles are
  CWD=1, stdin=2, stdout=3, stderr=4, other=5. Describe-kind tags are the
  current stable `SYS_CAP_DESCRIBE` kinds 1..=13. Each actual cap must match
  the descriptor's kind and rights exactly at runtime. CWD is a kind-12
  BadgedEndpoint with WRITE; CWD strings never grant access. Descriptor-index
  references must point to exactly one descriptor of the matching role.
- The header's IDs and execution facts are diagnostic metadata only; they
  never substitute for an `Image`, `Process`, directory, or stream capability.
- A capability descriptor records only the inherited child slot, a typed
  userspace role, `SYS_CAP_DESCRIBE` kind, and the exact rights mask that the
  runtime must observe. CWD and standard-stream references select descriptors;
  they never grant file access or stream authority themselves. Associations,
  display names, package paths and CWD text remain descriptive. Each real
  descriptor must be separately and explicitly granted by the manager.
- Standard streams are optional. The ABI reserves input/output/error roles,
  but it does not define a new kernel Stream object; their concrete capability
  protocol is ADR-0080's byte-stream gate. Until that gate is accepted and
  implemented, all three references are absent.

### Failure and compatibility

A malformed header, noncanonical offset, nonzero reserved/tail byte, missing
startup cap, unexpected object kind/rights/page count, or invalid descriptor
reference causes the runtime to exit before calling application code. The
manager must roll back its Process/cap/SharedRegion records on launch refusal.
Existing binaries continue through the unchanged legacy entry path; no Linux,
POSIX, implicit inheritance, or ambient path semantics are introduced.

## Consequences

- The startup page costs one SharedRegion page per ABI-v2 launch while retained.
  It is bounded by existing shared-region, process, cap, and frame admission;
  a manager preflights all of those resources before visible launch state.
- The caller-owned no-alloc codec is isolated in the dependency-light
  `arena-startup-abi` crate and re-exported by `arena-platform-core`; the entry
  runtime must not pull in package-signature crypto dependencies or allocate
  on the current initial stack. The Rust codec and independent Python oracle
  share a checked-in 4 KiB golden page and mutation controls; this is host wire
  evidence only.
- This ADR freezes wire and entry semantics, not a production app manager,
  general VM, multi-user-thread runtime or C library. The bounded heap and
  basic FS-base TLS have separate contracts in ADR-0084/0085. The `m12` QEMU
  suite boots an independently linked `arena-runtime` consumer on the exact
  kernel artifact; it verifies slot-0/slot-1 caps, the 4 KiB record, argv/env,
  entry/base, initial RSP/RFLAGS, four startup refusals, TLS address refusals
  and scheduler restoration, heap-slot collision preservation, and exact
  resource teardown (6/6). The production built-in path uses this codec and
  reusable runtime. The closeout M12 scale guest and affected M10/M11 guest
  regressions qualify the 32-session badge/slot integration. Installed-app
  registry launch and mature multi-window behavior remain separate gates.
