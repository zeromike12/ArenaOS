# ADR-0042 — Ring-3 fault isolation for managed production crash recovery

*Status: accepted and qualified. This crash checkpoint was partial; ADR-0043–0045 subsequently prove forced live stop, lifecycle refusals and active dependency probes, completing 8.0.*

## Context and decision

The existing exception handler halts ArenaOS on **every** unexpected CPU
fault, even a ring-3 #UD. An orderly `SHUTDOWN` request or a deliberate
`SYS_THREAD_EXIT` is not an unexpected crash. To prove recovery from an
actual fault in the production `netstackd`, the kernel must first isolate
an unhandled **user-mode** fault. This is a genuine correctness dependency
of the already-required crash criterion, not an extra milestone.

After the existing armed-exception test protocol, distinguish the saved
hardware frame's CPL. An unarmed user CPL3 exception on a real current
process records the vector, terminates only the faulting thread with a
non-success exit code, and sends the normal last-thread process-exit
notification. It must NOT return to the faulting instruction, panic the
kernel, sweep live caps while its address space is loaded, or `swapgs`
from the IDT path. The ring-3 exception stub did not enter through the
syscall stub's swapgs window: canonical GS is already active. The
manager's existing Process-cap reap then performs the ordinary server
teardown, failing any in-flight call with `STATUS_SERVICE_GONE`, freeing
the old process/record/caps, and triggering the normal bounded backoff
and restart. A CPL0 fault, a fault without a valid current process, and
#DF/NMI/#MC keep the existing kernel-fatal diagnostics (no hiding kernel
bugs). Preserve the existing expected-fault test path. Single CPU,
interrupts disabled on the exception path; no new exception-time
allocation or process table destruction.

For an **opt-in privileged** QEMU test, extend the production stack
protocol with a test-only `FAULT` opcode. Only the existing Power-holding
administrator has the production client endpoint (ADR-0040); no ordinary
application receives it. The stack executes a real `ud2` after accepting
that call but **before replying**. The blocked admin caller must receive
typed `STATUS_SERVICE_GONE` from reap; kernel logs must identify a ring-3
#UD rather than an orderly `SYS_THREAD_EXIT`. The same held endpoint then
reaches a fresh production child, old bearer authority is revoked, real
ARP reaches the wire, and a Power-gated resource snapshot returns to
its baseline. This is not a fake host transcript or M7 test-only child.
Do not auto-fault unattended boots or let the manifest request create
more authority. Keep the prior `stacktest` and `stackstress` gates.

The checkpoint ships with a complete historical suite, fresh final-EFI-
bound 100/100 boot qualification, and an extracted-and-booted QEMU
archive in the same commit. The special fixture proves one fault; normal
100-boot fixtures continue to exercise one orderly restart every boot.

## Limits

This decision proves an unexpected #UD from a one-thread production
child in an in-flight call. It does not prove forced live stop, Process-
cap refusal for foreign/self/driver targets, or proactive netd/rngd
health checks before respawn. Those are still the four-area 8.0 exit
criteria, not permission to mark 8.0 complete or start 8.1.
