# Phase 13 handoff — Native Runtime & Desktop Application Maturity

Phase 13 starts from the Phase-12 checkpoint recorded in
[FINAL-REPORT.md](FINAL-REPORT.md). Its focus is completing the native
application platform. Do not fold Linux execution into this phase; foreign ABI
work belongs to a later dedicated effort.

## Existing foundations to build on

- **Architecture and policy:** [ADR-0080](../adr/0080-phase12-application-platform-and-abi.md)
  keeps application policy, app identity, window policy, and future foreign
  syscall meanings in userspace. [ADR-0090](../adr/0090-desktop-badged-application-sessions.md)
  defines badge-authenticated built-in Desktop sessions.
- **Startup and exact authority:** `userspace/arena-startup-abi/src/` defines
  startup ABI v2; `userspace/arena-runtime/src/startup.rs` validates its page
  and live cap inventory; `userspace/desktop/src/bin/desktop.rs` constructs
  the built-in grant profile. See ADR-0083 and ADR-0086.
- **Lifecycle and handles:** `userspace/arena-process/src/` supplies
  `ChildProcess` and fixed-capacity `ProcessGroup`; runtime generation handles
  and actual capability wrappers are in
  `userspace/arena-runtime/src/{handles,capabilities}.rs`. Desktop session
  records store ProcessGroup handles and retain exact Process capabilities.
  See ADR-0087.
- **Native runtime bounds:** `userspace/arena-runtime/src/heap.rs` implements
  the bounded 32-page heap, and `userspace/arena-runtime/src/tls.rs` defines
  the FS.base contract. These are not general VM or multithreaded runtime
  facilities. See ADR-0084 and ADR-0085.
- **Desktop model and capacity:** `userspace/desktop/src/model.rs`,
  `userspace/desktop/src/desk.rs`, and `userspace/desktop/src/bin/desktop.rs`
  implement the window/session broker and 32 built-in one-window sessions.
  `userspace/desktop/src/service_wire.rs` and
  `userspace/desktop/src/session_auth.rs` carry the protocol and badge checks.
  ADR-0088 and ADR-0089 record the bounded capacity decisions.
- **Installed packages:** `userspace/arena-platform/src/` provides catalog,
  APB1 verification and policy models. `userspace/afs2/src/` provides the
  filesystem engine. `userspace/filesd/src/main.rs`,
  `userspace/packaged/src/main.rs`, `userspace/servicemgr/src/package.rs`,
  and `userspace/desktop/src/package.rs` implement the protected APB1 install
  path. ADR-0081/82/91 define the format, atomic AFS2 activation, and authority
  handoff. The boot-integrated installed-app registry/launcher remains future
  work.

## Work to complete

1. Integrate the verified installed-app catalog into boot and the Desktop
   launcher. Derive launch authority from a verified installed record and
   held Image capability; do not treat names, IDs, paths, or UI state as
   authority. Add All Applications browsing/search and running-state behavior.
2. Launch installed signed applications through a defined native executable
   contract. Preserve APB1 bundle policy and APKG v1 policy as distinct layers.
3. Add application associations, persistent defaults, and Open With. Transfer
   a selected document only as its exact filesd capability after broker policy
   authorizes the operation.
4. Separate app-instance, process-group, and window ownership. Add independent
   per-window surfaces/publication, multiple windows per application, and
   headless/helper processes with bounded lifecycle and failure policy.
5. Add generic streams, then stdin/stdout/stderr. Specify backpressure, partial
   I/O, EOF, cancellation, and process-death cleanup before application use.
6. Add process-wide user mappings and general VM operations if required, then
   a scalable heap. The current heap is fixed at 32 pages and its mapping model
   is single-thread-oriented.
7. Add user-thread creation, per-thread TLS, native synchronization, and
   process teardown only after shared address-space and allocator ownership
   are defined.
8. Measure broader app startup, concurrent windows, helper churn, package
   upgrade/restart, resource high-water, and repeated teardown on the guest.

Keep the qualified 32-session bound and 128-slot inventory visible in any new
budget. Preserve mutation-free refusal and exact capability accounting.

## Not part of Phase 13

Do not assume Linux syscall semantics, a foreign executable personality,
ProcessMemory, compatd, or Linux application compatibility. Define those in a
later phase after the native runtime and Desktop application model are mature.
