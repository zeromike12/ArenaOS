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
