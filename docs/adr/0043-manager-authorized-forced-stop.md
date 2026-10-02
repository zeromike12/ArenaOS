# ADR-0043 — Explicit, manager-owned forced live stop

*Status: accepted and qualified. This forced-stop checkpoint was partial; ADR-0044–0045 subsequently prove lifecycle-authority refusals and active pre-respawn probes, completing 8.0.*

## Context

An orderly exit and an unexpected #UD both leave the child dead before
`SYS_PROC_FINISH(slot, 0)`. Neither proves that the *manager* can force a
still-running production child down with its held Process/DESTROY cap.
Granting the administrator a copy of that Process cap makes the shell a
second lifecycle owner. Reusing the manager's event notification alone
is also unsafe: `netd` holds WRITE and could forge any control badge.
The manager-private restart timer must remain private (ADR-0040).

## Decision

The kernel's fixed full-fixture bootstrap installs **one additional,
distinct** notification: manager READ (slot 8), Power-holding shell
WRITE (slot 6). No driver, stack child or generic caller gets WRITE;
its object id must differ from the existing event, driver-ready,
backoff and restart-timer channels. The shell also gets WRITE to the
existing manager event notification (slot 5) as a **wake hint only**.
`netd` already has WRITE to it: the event badge cannot authorize a stop,
even if another actor forges the exact wake bit. Neither new shell cap
is a Process cap, and `Power` by itself does not authorize
`SYS_PROC_FINISH`. Only the manager receives a child Process cap.

Add `SYS_TRY_WAIT(slot)` (ABI 32), the nonblocking version of `SYS_WAIT`:
require the caller's Notification/READ cap, atomically take and clear an
already pending merged badge, return zero when none, never register a
waiter and never block. Wrong-kind, WRITE-only and absent caps fail
without touching a notification. A manager woken on the shared events
notification may use it to consume the distinct private admin request
without risking an untrusted actor blocking its monitor. The shell
notifies the private admin channel **first**, then the event wake;
pending badges survive either scheduling order. Other actors may forge
the event wake but cannot produce the private request. Private request
bits other than the exact STOP bit are refused, not treated as STOP.
The notification table limit rises explicitly 10 → 11 and the full
fixture proves a twelfth allocation is refused; no partial admin
capability is granted on a missing-driver boot.

The manager cross-checks its one unique *held* Process/DESTROY cap for
the current production pid and an observational live-thread snapshot,
then calls `SYS_PROC_FINISH(handle, 1)` **while the child is still live**.
The kernel rechecks the cap and refuses self and kernel-supervised
children, then tears down the process and in-flight IPC. This mode-1
path is not an `ARP_OP_SHUTDOWN` and does not ask the child to exit.
The manager logs forced stop only on kernel success, then uses the same
bounded policy, private backoff and live-cap replan as a natural exit.
If the child independently dies first, the manager follows the
ordinary mode-0 exit path; it must never claim to have forced a dead
child. A failed handle or stop is OFFLINE, not a silent fallback.

An opt-in `stackstop` command on the Power-holding shell first proves
real ARP wire work and obtains an rngd-backed UDP bearer, requests
private stop, then checks old-bearer refusal, new instance and fresh
wire work through the **same** held endpoint, and exact frame/spawn-
record/process accounting. A QEMU host test independently verifies
that the kernel observed a live child, the manager used mode 1, two
separately audited production children ran, and the old shell-held
endpoint had no lifecycle authority. Missing devices yield an honest
SKIP. Normal boot does not consume the manager's finite restart budget.

Run the full historical suite, a fresh final-EFI-bound 100/100 QEMU
qualification and an extracted-archive boot before committing the
source and deployable image together. At this checkpoint, 8.0 could
not yet close: lifecycle-authority refusals for forged/foreign/self/
driver handles and active dependency probes were unproven. They were
subsequently proven by ADR-0044/0045 and the full 8.0 qualification.

## Rejected shortcuts

- Hand the shell a Process cap, or add a pid-only `kill` syscall:
  violates the manager's sole lifecycle ownership.
- Trust a badge on the shared manager event notification as a STOP
  authorization: netd can forge it; badges are not authority.
- Reuse the private restart-timer notification as shell control:
  the administrator could forge a backoff timeout, invalidating the
  independence ADR-0040 explicitly requires.
- Change the normal 100-boot script into an automatic destructive
  forced-stop loop: a healthy unattended stack must remain resident.
