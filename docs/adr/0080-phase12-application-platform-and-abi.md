# ADR-0080 — Phase-12 application platform and ABI boundary

**Status: Accepted for Phase 12 (implementation gates remain open).**
**Date:** 2026-10-06. **Milestone:** Phase 12, application platform and ABI
foundations. **Audit:** [`docs/phase12/ARCHITECTURE-AUDIT.md`](../phase12/ARCHITECTURE-AUDIT.md).

## Problem

Phase 11 has a real userspace desktop, AFS2, a trusted file chooser, six
built-in application kinds, a measured twelve-session budget, and a small
signed dynamic-Image path. It does not yet have installed-application
identity, application instances/process groups, multiple windows per process,
a modern process-startup ABI, an app runtime, user threads/TLS, native
streams, or a foreign syscall execution boundary. The current bounds were
chosen and qualified for the twelve-session design; they are not safely
scaled by editing one array length.

The Phase-12 goal is to make a future application ecosystem and a later Linux
compatibility phase plausible without changing ArenaOS's native security
model into Unix semantics. Preserve every Phase-11 boundary and every
historical test.

## Decision

### 1. Keep applications, sessions, windows, and processes distinct

The kernel continues to know only generic processes, threads, address spaces,
capabilities, IPC, and VM mappings. It will not gain an `Application`,
`Window`, package-name, desktop-session, process-group, or POSIX descriptor
object merely to make the desktop convenient.

Userspace owns these separate concepts:

- **`AppDefinition`** — descriptive, versioned metadata from an installed,
  receiver-verified bundle. An ID/name, manifest, association, pin, or icon
  never grants launch or file authority.
- **`AppInstance`** — one launch lifecycle, with a fresh non-authoritative
  instance ID and policy/state owned by the application manager.
- **`ProcessGroup`** — zero or more exact held Process capabilities owned by
  that instance, including the primary process and explicitly requested
  helpers. Membership is recorded only after a capability-authorized spawn;
  a numeric PID or app ID cannot add a process.
- **`WindowSet`** — zero or more compositor-owned windows whose surfaces are
  offered through explicit capabilities. A window handle is presentation
  identity, not process or app authority. A process may be headless or own
  multiple windows; an app instance may have multiple processes.

Lifecycle policy, discovery, launch intent, associations, and running-state
presentation remain userspace responsibilities. The desktop broker remains a
trusted policy mediator for windows and chooser-issued document capabilities.

### 2. Registry, file association, and launcher are metadata clients

A userspace installed-app registry is reconstructed from immutable installed
bundle manifests, not a compile-time six-app array. It exposes descriptive
records and content-handler candidates. User favorites and default
associations are persisted as AFS2 user data. `Open With` and a default
association select a descriptive app ID only; a document capability is
separately held by the chooser/desktop and explicitly offered to the selected
instance through the existing capability mediator.

The All Applications launcher and search are userspace views of that
registry. The dock contains pinned/favorite apps plus actually running apps;
it is not the package inventory. A launch must resolve the selected definition
to an installed, signature/policy-verified version and a live executable
capability before spawn. Missing/uninstalled/corrupt definitions fail closed.

### 3. APB1 is a new signed multi-file format; APKG v1 is unchanged

Name the new format **APB1 (ArenaOS Application Bundle v1)**. It is not an
extension or reinterpretation of APKG v1. APB1 will use the existing
receiver-verified Ed25519/SHA-256 trust closure and explicit signer/policy
rules; no package name, file name, app ID, registry entry, or Stage response
is a signature or install authority. Its canonical signed metadata binds the
versioned application manifest and a sorted file table containing file type,
canonical relative path, exact size, and SHA-256 for each file. File payloads
are individually streamed and verified with a bounded scratch buffer; the
whole application is never required in a fixed kernel or installer buffer.

The format/install design must define finite limits for metadata, file count,
per-file and total installed bytes; reject path traversal, duplicate/colliding
paths, noncanonical encodings, unlisted bytes, and bad hashes; and install an
immutable version transactionally to AFS2. An active version becomes visible
only after complete receiver-side verification and atomic AFS2 activation.
`APKG` v1 parsers, signatures, policy, and all legacy tests stay byte-for-byte
compatible. APB1 adds no authority to the kernel and does not reuse the
Phase-8 coarse manager marker for a new purpose without a separate explicit
invariant.

### 4. Native process startup ABI v2 is capability-native

A modern native process receives one bounded, versioned startup block built by
the trusted spawn authority. The final wire layout/limits are frozen by a
follow-on ABI ADR and independent guest tests; it must include an ABI version,
`argc`/`argv`, environment entries, descriptive app-instance metadata,
explicit initial capability descriptors, optional standard-stream handles,
a held current-directory capability plus optional display-only text,
executable/base metadata, page-size/clock facts, and entropy only when the
launch authority explicitly supplies an RNG service capability.

No path string is authority. No hidden allocation occurs before the entry
block becomes available. The block remains in a validated user mapping, and
cap slots/rights are the only native authority descriptors. Launch refusal
must be bounded and leave no process, frame, capability, record, or ID mutation.
The exact initial x86-64 register state, RSP alignment, startup-block layout,
TLS state, and rejection limits are part of the ABI contract, not compiler
accident.

### 5. Native runtime stays distinct from POSIX

Build `arena-runtime` above `arena-lib`, and `arena-ui`/toolkit above the
runtime. The runtime may provide native heap/VM, TLS, user-thread lifecycle,
mutex/condition/once, explicit child launch/wait, byte streams/pipes, and an
optional integer handle table. None is named or specified as POSIX for
convenience. Integer handles are application-local references to already held
ArenaOS runtime objects; they carry no authority without those underlying
capabilities. Native low-level APIs remain capability-oriented.

Where current kernel mechanisms are insufficient, introduce the smallest
generic memory/thread/wait object API through its own ADR. Each operation is
bounded, process-owned, cleaned on teardown, and visible in resource
accounting. Do not use Unix `mmap`, `futex`, file-descriptor, signal, or
implicit `fork` semantics as the native specification.

### 6. Child authority is explicit; application manager retains lifecycle caps

A native app asks a userspace application manager to launch an allowlisted
installed helper. The manager resolves a helper to a verified immutable Image
capability and invokes the existing spawn mechanism or a reviewed additive
variant with an explicit rights-attenuated capability list. There is no
ambient inheritance, process-name lookup, or caller-supplied PID authority.
The manager retains the original Process capability for every child it owns;
exit, wait, stop, and reap operate on those held capabilities and typed status
facts. A child cannot claim membership in a ProcessGroup by naming an app or
PID.

### 7. Scale is a resource budget, not a collection of larger constants

The qualified twelve-session assumptions are retired only by an explicit
budget and integrated guest proof. The platform must demonstrate at least 16
application/process instances and 32 ordinary windows, with headless helpers,
multiple windows per app, and resource reclamation. Resource admission must
preflight and account for processes, user threads, spawn records, broker cap
slots, endpoints, notifications, SharedRegion slots/pages/maps, private
publication snapshots, page-table frames, physical frames, timers, watches,
and installed Images. Rejections happen before visible mutation and return a
specific capacity result.

Prefer variable, per-window surface allocations sized to the declared
window/surface bounds, with private authenticated publication snapshots, over
reserving a maximum scanout-sized pair for every process. Any different design
must include measured 800×600 and 1024×768 budget receipts and a proof of
mutation-free refusal. The single-page SharedRegion registry, process/thread
limits, or desktop arrays may change only after the end-to-end inventory is
known.

### 8. Foreign ABI is an explicit, launch-selected syscall proxy personality

Native processes always use ArenaOS's native syscall dispatch. A trusted
launcher may create a process in a distinct **foreign personality**. That
process cannot select or later change its personality itself and can never
fall through to native syscall implementations. At the generic kernel
boundary, a foreign syscall is only a bounded request tuple (number, six
register arguments, caller-bound identity and cancellation token) sent to a
held userspace proxy authority. The kernel does not interpret foreign syscall
numbers, names, flags, errors, file operations, signals, or threading
semantics.

The userspace proxy server (generic name `compatd`) owns translation policy
and uses separately granted ArenaOS capabilities. Replies carry a result and
an explicitly designed action enum; they do not grant arbitrary signal or
kernel action authority. A dead/revoked server fails its foreign clients
closed. Exact request, reply, cancellation, backpressure, caller identity,
server authority, and restart rules require a follow-on proxy ADR before code
is wired into syscall dispatch.

The foreign process's user memory is not exposed as arbitrary physical memory.
A future proxy may receive an exact per-child `ProcessMemory` capability with
bounded checked copy operations, READ/WRITE attenuation, lower-half range
checks, and revocation at child death. The capability names the exact held
process object, never a PID string. It must not provide access to another
process or kernel memory. A kernel-only implementation detail is not a license
to broaden a compat server's rights.

### 9. Do not implement Linux semantics in Phase 12

Phase 12 may demonstrate a tiny deliberately non-native test ABI through
`compatd` and prove that native dispatch does not accidentally accept it. It
must not grow a Linux syscall table in the kernel or claim Linux compatibility.
No Linux filesystem pseudo-trees, signals, futex, epoll, io_uring, ioctl,
PT_INTERP/ld-linux, glibc/musl, X11/Wayland, Wine, or containers are in scope.

## Options considered

1. **Turn the kernel process into an application/session and add fd/syscall
   semantics there.** Rejected: entangles presentation and package policy with
   ring 0, makes IDs/strings ambiguous authority, and pollutes the native ABI
   with exactly the foreign semantics a later compatibility runtime should
   translate.
2. **Keep six kinds and raise the session count.** Rejected: does not provide
   an installed registry, multi-window/process ownership, native startup
   contract, or a measurable multi-resource budget; the audit shows that
   table/memory assumptions conflict.
3. **Add userspace app definitions/instances/groups over existing generic
   processes/capabilities; keep one generic foreign-proxy kernel mechanism.**
   Selected: policy remains inspectable/restartable in userspace; native
   authority remains capability possession; the future foreign ABI has one
   explicit boundary without contaminating native dispatch.

## Downsides accepted

- The application manager and bundle verifier become policy TCB components;
  their exact held capabilities and failure/rollback path must be audited.
- A proper multi-window surface protocol is more work than duplicating
  windows; it is required to avoid accidental shared-raster aliasing.
- Runtime ABI v2 is a new long-lived contract. It will be frozen only after
  real independent payloads exercise it.
- A foreign proxy costs a context transition and bounded queueing; performance
  is subordinate to isolation and fail-closed behavior in this proof phase.

## Consequences and revisit triggers

- ADR-0002/0006 remain controlling: capabilities, not names/PIDs/app IDs,
  authorize. Native syscalls stay native; the foreign personality is a client
  of ArenaOS mechanisms.
- ADR-0053/54/55 remain controlling for existing APKG v1 trust/staging,
  installed/active records, and Image registration. APB1 must be a distinct
  format and verifier/install path with explicit updates to those records.
- ADR-0075's twelve-session capacity is historical and must not be described
  as generalized application-platform capacity until measured anew.
- Later ADRs must make exact wire layouts and fixed limits for APB1, startup
  ABI v2, user VM/TLS/threads, streams/handles, child groups, PIE/ASLR, and
  syscall proxy/ProcessMemory explicit before their implementation is used by
  an untrusted guest.
- Revisit only if an actual mechanism cannot remain in userspace without
  defeating the capability boundary, or if the measured resource budget cannot
  meet the Phase-12 16-instance/32-window requirement under the 512 MiB QEMU
  profile.
