# ADR-0071 — Endpoint-bound notifications and a per-process timer quota

Status: accepted; implemented on `arena/phase11-desktop-maturity` (Phase 11.0).

## Problem

A userspace server could wait either for IPC (`SYS_IPC_RECV`) or for a
notification (`SYS_WAIT`: timers, interrupts, child exit), never for both.
The Phase-10 compositor, which must react to client requests, input, child
exit and animation deadlines, therefore polled its endpoint with
`SYS_IPC_TRY_RECV` on every 10 ms tick (about 160–195 wakes/s measured with
an idle desktop) and still added up to a tick of latency to every request
that arrived while it slept.

Separately, the 32-entry kernel timer table had no per-process bound: any
holder of a notification capability with WRITE could arm every timer and
starve all other processes of timeouts.

## Decision

### Endpoint-bound notification

`SYS_ENDPOINT_BIND(endpoint slot, notification slot, badge)` (call 46) and
`SYS_ENDPOINT_UNBIND(endpoint slot)` (call 47).

* **Authority is capabilities only.** Bind requires READ on the endpoint
  (the serve side: a WRITE-only client cannot redirect its server's wakes)
  and READ and WRITE on the notification (the binder can already wait on
  and signal it, so binding confers nothing new). Unbind requires READ on
  the endpoint. No PID, name or window identity is consulted. Reserved
  arguments must be zero.
* **Semantics.** When a CALL is enqueued on the endpoint while no server
  thread is parked in RECV, the kernel ORs the badge into the bound
  notification and wakes its waiter. A server therefore waits on one
  notification for "timer OR IPC OR anything else" and then drains its
  endpoint with `SYS_IPC_TRY_RECV`. Delivery to a server already parked in
  RECV is unchanged and raises no signal. Binding while calls are already
  queued signals once immediately, so no queued request depends on a later
  unrelated wake. The badge is sticky pending state, so a call that arrives
  between a drain returning BUSY and the next wait is never lost.
* **Bound.** At most one binding per endpoint; binding again replaces it.
  A notification may be bound by several endpoints with different badges.
  No table grows: the binding is one field in the fixed endpoint object.
* **Cleanup.** The binding is removed when the endpoint is destroyed, when
  the notification is destroyed (every endpoint referencing it is scanned
  and unbound under IF=0), and when the endpoint is orphaned by the death of
  the process holding its serve side (`fail_calls_for_server`). A successor
  taking over an orphaned endpoint must bind its own notification.
* **No stale authority revival.** Notification objects now carry a
  generation advanced on every mint and destroy. A binding records
  `(nid, generation, badge)` and is honoured only while the live object's
  generation still matches, so a destroyed index re-minted for a different
  purpose can never be signalled even if a cleanup path were missed (this
  is defence in depth; the RED controls below remove each cleanup path
  individually and the oracle observes the stale binding directly).

### Per-process timer quota

`MAX_TIMERS_PER_PROCESS = 4` inside the shared 32-entry table. The bound is
measured: across the complete historical suite and the desktop, no process
holds more than three armed timers at once (timertest's merge proof); four
leaves one spare and guarantees that at least eight processes can always
arm. An over-budget arm is refused with the new typed status
`STATUS_QUOTA = -7`, distinct from the table-full `STATUS_BUSY`. The quota
is derived from the timer table itself (live entries owned by the pid), so
it cannot drift, outlive a slot, or be inherited by a later process: timers
are released by owner when a process is destroyed, as before. The kernel
records refusals and the per-process high-water mark.

## Consequences

* The compositor binds its request endpoint to its own notification
  (badge 2), receives child exit on the same notification (badge 4) and
  arms a timer only for real deadlines (motion frames, the next uptime
  second, notice expiry). Built-in clients arm no timer at all unless they
  have time-driven work (Monitor's sampling).
* Measured on the same host (TCG, `tools/profile_desktop.py`): compositor
  wakes with an empty desktop 195/s → 1.06/s; with six idle applications
  163/s → 4.69/s; static idle clients 0 wakes/s (Monitor 1.95/s, its sample
  rate).
* Proofs: boot suite `m11` (bound signal; notification destroy and
  same-index re-mint never signalled; endpoint destroy; orphan and
  takeover; timer quota; handoff order); ring-3 timertest proves the fifth
  simultaneous timer returns `STATUS_QUOTA`; `tools/test_m11_event_red.py`
  removes, one at a time, the bound signal, the notification-destroy
  unbind, the orphan unbind and the quota check, and requires the real
  guest to report the matching failure, then proves the byte-exact restored
  source GREEN.
