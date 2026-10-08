# Phase 12 compatibility handoff

This record separates compatibility that is qualified from compatibility that
has not been implemented. The final hashes and exact suite receipts are in
[FINAL-REPORT.md](FINAL-REPORT.md).

## Qualified compatibility boundaries

| Surface | Status |
|---|---|
| Historical ArenaOS phases 1–11 | Preserved by the historical and affected guest regression suites listed in `FINAL-REPORT.md`. |
| APKG v1 package records and policy | Historical wire/record semantics remain unchanged and are covered by APKG v1 host and guest tests. APKG v1 package policy is distinct from APB1 bundle verification/installation. |
| APB1 v1 | Canonical signed multi-file native bundle format with a separately qualified protected AFS2 install handoff. APB1 does not imply an application registry entry or automatic launch. |
| Native application startup | ABI v2 is the current built-in native startup contract. It validates a bounded startup record and the exact live cap inventory. It is not a POSIX ABI. |
| Existing ELF execution | ArenaOS's native loader contract remains the accepted static x86-64 native subset. No dynamic linker or general PIE/ASLR guarantee is claimed. |

## Explicitly absent

The Phase-12 tree does not provide a foreign ABI syscall personality, generic
syscall proxy, ProcessMemory capability, `compatd`, Linux syscall semantics,
Linux filesystem/process/thread behavior, or Linux application compatibility.
A foreign executable must not be assumed to run because ArenaOS has a native
ELF loader or a signed-package format.

Native streams, general VM, user-created multithreading, multi-window-per-app
production behavior, and a scalable heap also remain deferred; their absence
limits the native runtime independently of foreign ABI support.

## Later compatibility phase

After Phase 13 matures the native runtime, a later dedicated compatibility
phase may evaluate a userspace-only syscall personality and proxy. That work
requires its own ADR for trusted personality selection, bounded request/reply
semantics, cancellation and server death, child-scoped memory access, pointer
validation, and negative controls. Linux syscall meanings must not enter the
kernel's native syscall dispatcher. No compatibility claim should be made
until those mechanisms pass guest qualification.
