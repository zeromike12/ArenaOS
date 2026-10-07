# ADR-0086: Exact live-capability inventory at native entry

Status: Accepted; exact inventory is guest-qualified for the 128-slot process cap table.
Date: 2026-10-05
Authors: ArenaOS project / Phase 12

## Milestone context

Phase 12.4, supplementing [ADR-0083](0083-native-startup-abi-v2.md). This
record adds an enforcement mechanism for ADR-0083's exact startup-capability
contract; it does not change the frozen ARST v2 page bytes or the existing
`SYS_CAP_DESCRIBE` descriptor format. ADR-0088 later widens the mirrored
capability-space bound from 64 to 128 for measured Phase-12 desktop scale; the
occupancy query and entry gate continue to cover every slot.

## Problem

The reusable native entry gate compares every capability named by ARST v2
against the caller's held object kind and rights. `SYS_CAP_DESCRIBE` is
intentionally partial: a held object whose kind cannot be described and an
empty slot both return an error. Looking only at the listed descriptors
therefore cannot prove the negative space of the startup grant. An undeclared
capability could remain live, including a kind that the descriptor syscall
must not expose.

The application contract requires an exact grant, not merely that listed
capabilities are valid. Slot numbers and descriptive IDs remain non-authority;
this check must inspect the caller's actual bounded capability space without
turning the runtime or kernel into an ambient-capability registry.

## Options considered

1. **Continue validating only listed descriptors.** Rejected: it accepts extra
   live authority and cannot distinguish an empty slot from an occupied but
   undescribed kind.
2. **Make `SYS_CAP_DESCRIBE` disclose every capability kind.** Rejected: this
   expands metadata disclosure and changes a separate descriptor ABI just to
   answer a presence question.
3. **Add a caller-local, metadata-free occupancy query and scan the frozen
   table.** Chosen: it reveals only whether a slot in the caller's own space is
   occupied; the existing describe call remains the authority-kind/rights
   check for the exact listed rows.

## Decision

Add native syscall **54**, `SYS_CAP_OCCUPIED(slot)`, with the following frozen
contract:

- It queries only the calling process's capability space and a slot in the
  existing `CAP_SLOTS = 64` bound.
- The slot is the sole argument; all five reserved argument registers must be
  zero. An out-of-range slot or caller without a process capability space is
  rejected.
- It returns `0` for empty and `1` for occupied. It reveals no object kind,
  identity, rights, physical/device address, or other metadata; it cannot copy,
  invoke, mutate, or mint a capability.
- The number and 64-slot bound are mirrored in `userspace/abi.rs`. A future
  change to either bound is a separate ABI decision, not an implicit capacity
  increase here.

After validating and consuming the one-page startup transport in slot 0, the
native runtime scans every slot in the cap space. For each slot, occupancy must
match the frozen ARST v2 descriptor list: slots 1 through the listed count
must be occupied, and slot 0 plus every other slot must be empty. Each listed
occupied slot is still checked with `SYS_CAP_DESCRIBE` for exact kind and
rights. Any occupancy-query error, missing listed cap, extra occupied slot,
undescribed listed kind, or kind/rights mismatch refuses startup before the
application closure runs. Existing startup cleanup and parent-owned child
teardown remain responsible for releasing resources after refusal.

The check is native ArenaOS startup policy only. It adds no Linux or POSIX
semantics, and no app name, PID, path, window, integer handle, or descriptor
value becomes authority. ARST v2 bytes and APKG v1 bytes are unchanged.

## Evidence

- `arena-runtime` host tests cover exact valid inventory, missing listed caps,
  unexpected slot 0, extra caps (including a kind whose description is
  unavailable), descriptor mismatch, and occupancy/description query errors.
- The independent M12 guest adds `unlisted_capability_red`: the kernel grants a
  live `MemoryPool/WRITE` cap in undeclared slot 2 alongside the one listed
  Notification. The runtime refuses before the application marker, and the
  parent verifies exact resource return. The full M12 suite passes 7/7,
  including five startup RED controls and the reserved heap-slot collision;
  M1–M7 and M11 regression markers pass in the same boot.
- The current `arena-runtime` startup/heap/TLS host suite passes 13/13 (the
  separate `arena-process` handle/process suite adds 7/7); runtime Clippy is
  clean and the independently linked `arena-startup-proof` release image
  builds. The targeted M12 QEMU boot exits cleanly through the kernel reset
  path.

## Consequences

An accepted native startup page now describes the complete initial authority
set, rather than a subset. A future capability kind automatically participates
in the occupancy scan even if `SYS_CAP_DESCRIBE` deliberately does not expose
it; an app must receive such authority through an explicit descriptor or not
at all. The query exposes one bit per caller-owned slot, which is deliberately
less information than a capability descriptor and does not alter kernel
capability semantics. Production launcher integration and post-entry runtime
capability management remain separate Phase-12 work.
