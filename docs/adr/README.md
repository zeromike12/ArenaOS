# Architecture Decision Records

An ADR records a significant architectural decision: the problem, the options
considered, the choice, the reasoning, the downsides we accepted, and the
future implications. Decisions are not allowed to live only in conversation
history or commit messages.

Rules:

- One ADR per decision. Small decisions do not need one; architecture-defining
  ones do.
- ADRs are numbered sequentially and never renumbered.
- An accepted ADR is only changed by a *new* ADR that supersedes it (status
  line updated, both kept).
- Status values: `Proposed`, `Accepted`, `Superseded by ADR-XXXX`,
  `Deprecated`.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-implementation-language.md) | Implementation language: Rust (stable, `no_std`) | Accepted |
| [0002](0002-kernel-architecture.md) | Kernel architecture: capability-based hybrid microkernel | Accepted |
| [0003](0003-boot-strategy.md) | Boot strategy: kernel image is a UEFI application | Accepted |
| [0004](0004-no-third-party-runtime-crates.md) | Zero third-party runtime crates; own UEFI bindings | Accepted |
| [0005](0005-testing-strategy.md) | Testing: automated QEMU boot tests with machine-checkable serial markers | Accepted |
| [0006](0006-abi-philosophy.md) | Kernel/user ABI philosophy: capability invocation | Accepted (direction; register-level detail due at M4) |
| [0007](0007-frame-allocator.md) | Physical frame allocator — flat bitmap over conventional memory | accepted |
| [0008](0008-kernel-address-space.md) | Kernel address space — dual-view paging with per-section W^X | accepted |
| [0009](0009-kernel-heap.md) | Kernel heap — host-tested free-list core, frame-backed chunks, always-on guards | accepted |
| [0010](0010-synchronization-primitives.md) | Synchronization — spinlock with owner tracking, irqsave critical sections | accepted |
| [0011](0011-boot-split-kernel-entry.md) | Boot split — ExitBootServices, kernel entry, and the reclaimed timer chain (PIT on IOAPIC pin 2) | accepted |
| [0012](0012-kernel-threads-context-switch.md) | Kernel threads and the context switch — callee-saved frame, no FPU state (build-enforced) | accepted |
| [0013](0013-preemptive-scheduling.md) | Preemptive scheduling — nested cooperative switch from the timer-tick hook, IF=0 invariant | accepted |
| [0014](0014-processes-ring3-syscall.md) | Processes, ring 3, and the syscall boundary — address-space objects, syscall/sysret, SMAP/SMEP (+ M3.3b addendum: kernel-half MMIO alias rule) | accepted |
| [0015](0015-capability-spaces.md) | Capability spaces — slots, rights, attenuation-only delegation, and the first gated invokes | accepted |
| [0016](0016-executable-format-loader.md) | Executable format and image loader — ELF64 container, ArenaOS strict-subset semantics, W^X segments | accepted |
| [0017](0017-syscall-abi-v1.md) | Syscall ABI v1 — six-register encoding, typed status codes, frozen call-number registry | accepted |
| [0018](0018-ipc-v1.md) | IPC v1 — endpoints, synchronous call/reply, badged notifications, capability transfer in messages | accepted |
| [0019](0019-spawn-protocol-v1.md) | Spawn protocol v1 — process creation from image capabilities, explicit attenuating inheritance, exit-badge notifications | accepted |
| [0020](0020-console-input-minimal-shell.md) | Console input and the minimal shell — COM1 RX line discipline, SYS_CONSOLE_READ/PROC_LIST/SHUTDOWN, Power capability, boot hand-off to the initial service | accepted |
| [0021](0021-driver-substrate.md) | Driver substrate — owned Untyped frame caps, self-map windows, kernel-minted Mmio caps, IRQ relay vectors, kernel-side PCI enumeration as policy | accepted |
| [0022](0022-userspace-block-service.md) | The userspace block service — storaged's ring-3 VirtIO driver, the zero-copy block protocol over IPC v1, owned vs. lent Untyped caps, SYS_IRQ_RELAY/DEV_INFO, poison-shutdown lifecycle | accepted |
| [0023](0023-afs1-filesystem-service.md) | AFS1 — the own-design filesystem v1 (extent data, CoW transactional metadata, ping-pong commit), IPC v1.1 inline messages, the block protocol's in-frame offset, fsd's forwarded-cap zero-copy chain, host-side mkfs + post-boot disk verification (+ M5.4 addendum: persistence, the crash model, fh-as-capability, transactional UNLINK) | accepted |
| [0024](0024-virtio-net-driver.md) | virtio-net in ring 3 — `netd`, the link-only driver server: two packed virtqueues under the cap-slot budget, two MSI-X relay badges, zero-copy TX over LENT caps, inline-message RX v1, the ARP link proof against slirp, order-independent DEV_INFO discovery, graceful absence (honest SKIP) | accepted |
| [0025](0025-shared-virtio-core-rngd.md) | The shared virtio core (`userspace/virtio.rs`, ADR-0024's third-driver rule executed) and `rngd`, the zero-copy entropy service: typed stage errors mapped per driver, Packed/Split ring layouts, RNG_GET filling a caller-LENT frame by device DMA, the variance proof (no zeros, no constants, draws differ), bare `virtio-rng-pci` fixture via QEMU's rng-builtin default, registry caps raised to 12 | accepted |
| [0026](0026-virtio-input-keyboard.md) | The virtio-input keyboard (`inputd`, registry image 10) and how a ring-3 driver reaches the shell: transport chosen as virtio-input-hid (modern-only 0x1052; i8042 PS/2 recorded as the rejected fallback), a device-writable event queue kept stocked against QEMU's whole-batch drops, and — instead of the roadmap's "switch the shell off serial" — a `CapObj::ConsoleInput`-gated `SYS_CONSOLE_PUSH` feeding the kernel's existing line discipline, so serial and keyboard are both live and the shell is unchanged; zero-length push as the capability probe (one image, two modes); QMP as the harness's second channel | accepted |
| [0027](0027-virtio-console-channel.md) | The virtio-console channel (`consoled`, registry image 12): what makes a port a console — the kernel keeps ONE line discipline and drivers attach CHANNELS to it, inbound through ADR-0026's `ConsoleInput` gate and outbound through a new `ConsoleOutput`-gated output MIRROR (`SYS_CONSOLE_ATTACH` / `SYS_CONSOLE_PULL`); MULTIPORT declined so one port needs two queues and no control protocol; buffers posted after DRIVER_OK because QEMU pauses a chardev the guest cannot yet read; the desktop terminal decided AGAINST multiplexing over this. Plus two latent kernel bugs it exposed: relay stubs clobbering rcx before saving it, and boot TSC calibration halting the machine on a stalled host | accepted |
| [0028](0028-supervised-restart.md) | Supervised restart: a dead service becomes a typed `STATUS_SERVICE_GONE` for its in-flight callers (endpoints served are found by CAPABILITY, not bookkeeping), a blocked process can finally be KILLED (`sched::kill_threads_of`, with kernel references released first because waking a corpse halts the machine), and `supervise.rs` respawns the image with its grant list REPLAYED while the endpoint — and therefore every client's capability — survives untouched; restart bound, no back-off, no state recovery, supervisor kept in the kernel because minting Mmio is the one authority never delegated. Closes ADR-0025's spawn-record GC debt | accepted |
| [0029](0029-timer-facility.md) | Timers for ring 3, decided before the first protocol: `SYS_CLOCK_NOW` plus `SYS_TIMER_ARM`/`CANCEL`, delivering a BADGE on a notification the caller already holds — so 'device interrupt OR client request OR timeout' is an ordinary `SYS_WAIT` with one more bit and needs no second thread, no new blocking primitive and no new capability kind (WRITE on the notification is the gate, as with `SYS_IRQ_RELAY`). Relative delays, one-shot, ~10 ms granularity stated as NOT BEFORE, owned and swept at `proc::destroy` (the fourth such sweep). Plus `tick.rs`, a dispatcher for deferred tick work. Polling loops are a named anti-goal of Phase 7 | accepted |
| [0030](0030-network-stack-service.md) | ARP as a service: protocol state lives in `netstackd` (image 17) and NEVER in the NIC driver — structurally, since the stack holds no device capability at all — because a driver's job is to survive its device and be restartable while a stack's job is to hold state across time. `NET_OP_RECV` gains a DEADLINE enforced by netd, since a client blocked in `SYS_IPC_CALL` cannot observe its own timer (ADR-0029 erratum); ARP cache aging is by clock, not timer, because nothing must happen when an entry expires; retry only what is idempotent. Proven on the wire: real resolution, a cache hit that moves no packets, and a silent address that TERMINATES in UNREACHABLE | accepted |
| [0031](0031-ipv4-icmp-demux.md) | IPv4 + ICMP echo, and the receive DEMULTIPLEXER they hang from: with one protocol an operation could read the wire itself, with two that discards other protocols' frames and times out the operation waiting for them, so recognising a frame is one job in one place with counters that must account for every frame. IPv4 kept to the smallest honest amount (no options, no fragmentation, no routing) but done properly — both checksums VERIFIED on receive, replies matched on identifier AND sequence, echo requests deliberately unanswered rather than shipped untested; the layering visible in the failures (UNREACHABLE vs NO_REPLY) | accepted |
| [0032](0032-frames-larger-than-a-message.md) | Frames larger than one IPC message: netd STAGES a received frame in the ring where the device put it and serves it by offset, instead of dropping anything over 64 bytes — a limit that silently decided which protocols could exist (a DNS answer does not fit). Not zero-copy, and says so. Proven by making the EXISTING proof require it (the echo payload went 8 → 200 bytes, verified byte-for-byte with a position-dependent pattern). Also records why this came before UDP, and why a port must NOT be a kernel capability | accepted |
| [0033](0033-udp-and-authority-by-possession.md) | UDP, and a port handle that is AUTHORITY rather than identity: `BIND` returns an rngd-drawn 64-bit token and possession of it is the right to use the port — a capability one layer below the kernel's, with deliberate passing as delegation. Records two wrong instincts first (a kernel `CapObj::UdpPort`, which would teach the kernel UDP; then per-client endpoints, which confused authority with identity — C's correction). Random on purpose: a guessable handle is authority by arithmetic, and without entropy the stack REFUSES to bind. Proven with a real DNS query matched on our transaction id | accepted |
| [0034](0034-dns-resolver-and-udp-continuations.md) | DNS A lookup, UDP continuations, checksums and bearer revocation | Accepted |
| [0035](0035-tcp-active-open.md) | Bounded TCP active open and disjoint netd RX/TX badges | Accepted |
| [0036](0036-native-networking-api.md) | Native userspace API with explicit bearer delegation and real-wire proof | Accepted |
| [0037](0037-service-manager-authority.md) | Phase 8.0 service manager authority, manifests, lifecycle ownership and bootstrap | Accepted |
| [0038](0038-service-manager-bootstrap-readiness.md) | Fixed manager boot grants, device-originated readiness badges, and fail-closed inventory bootstrap (8.0 partial) | Accepted |
| [0039](0039-managed-stack-startup.md) | Manager-owned initial stack startup, bounded ready signal, child cap audit and qualified per-commit QEMU bundle | Accepted |
| [0040](0040-managed-restart-exercise.md) | Bounded Process-cap reap/restart, privileged opt-in real-wire restart and its remaining negative-space gates | Accepted |
| [0041](0041-repeatable-manager-accounting.md) | Power-gated kernel resource snapshot, three exact-accounting production restarts and exhausted budget | Accepted |
| [0042](0042-user-fault-managed-crash.md) | Isolate unexpected ring-3 #UD, fail in-flight production IPC and recover through manager-owned Process-cap reap | Accepted (partial 8.0) |
| [0043](0043-manager-authorized-forced-stop.md) | Separate private admin request from forgeable event wake; only the manager's Process cap can force-stop a live production child | Accepted (partial 8.0) |
| [0044](0044-process-cap-lifecycle-refusals.md) | Real held Process references, protected/foreign refusal, kernel-owned bootstrap provenance, successful user-child reap | Accepted |
| [0045](0045-active-dependency-probes.md) | Bounded active netd MAC/rngd device-entropy probes before each managed spawn, fault/stall fail-closed fixtures and deferred driver-fault teardown; completes 8.0 | Accepted |
| [0047](0047-service-diagnostic-authority.md) | Receiver-verified boot-granted marker for destructive IPC; legacy poison refusals, managed #UD/stall proof and IPC-landed reference cleanup | Accepted |
| [0046](0046-transactional-configuration-store.md) | 8.1 bounded single-key immutable generations, receiving-service update marker, crash boundaries, numeric resources and DEGRADED barrier; explicitly scoped corruption guarantee; no GC | Accepted (8.1 complete) |
| [0048](0048-permission-manifests-and-grant-workflow.md) | 8.2 durable one-endpoint permission mediator: separate request/approval/grant, 128-bit revocable bearer, immutable `perm8-*` policy and broker restart/crash gates | Superseded by ADR-0049 for worker witness only (8.2 complete) |
| [0049](0049-bounded-permission-readiness-witness.md) | Two-grant bounded PING worker: mediator client and existing private manager-result notification; success + exit without deadline before READY | Accepted (8.2 complete) |
| [0050](0050-dead-ipc-caller-lifecycle.md) | General IPC caller sweep and broker-first deadline; four-state staged-cap/typed-refusal M4 proof plus caller-first real guest regression; qualified partial integration | Accepted (8.2 subsequently complete) |
| [0051](0051-filesystem-absence-proof.md) | Distinct receiver-verified fsd shutdown marker and kernel-root exited-service reap to orphan the real FS endpoint; typed absence and restoration | Accepted (8.2 complete) |
| [0052](0052-standard-userspace-client-libraries.md) | Separately linked no_std syscall/IPC/AFS1/native-network clients, checked returned-cap semantics, two independent guest consumers each; 42/42 + fresh final-EFI 100/100 + extracted boot | Accepted (8.3 qualified complete) |

## Template

```markdown
# ADR-NNNN: Title

Status: Proposed | Accepted | Superseded by ADR-XXXX
Date: YYYY-MM-DD
Milestone context: which phase this decision belongs to

## Problem
What forces a decision? Why can't we defer it?

## Options considered
Option A / B / C with honest trade-offs.

## Decision
What we chose, stated unambiguously.

## Reasoning
Why, tied to project goals (vision, 20-year maintenance, testability).

## Downsides accepted
What this costs us.

## Future implications
What this constrains or enables later; what would trigger revisiting.
```
