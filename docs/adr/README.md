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
