# ADR-0029 — Timers for ring 3: the timing contract, decided before TCP

*Milestone 7.0. Status: accepted.*

## Problem

Phase 7 builds ARP, IPv4, ICMP, UDP, DNS and TCP. Every one of them
needs to know that something did **not** happen within some time:
retransmission, ARP aging, DNS timeout and retry, connection
establishment, and later TIME_WAIT. Ring 3 had no way to measure or
wait for time at all.

ADR-0028 had already recorded the same hole from the other side: a
client blocked in `SYS_IPC_CALL` cannot give up on a service that has
stopped answering, because it has no timeout to give up *with*.

The failure mode if this is left until the protocols need it is
specific and well known: a stack grows polling loops and busy-waits
that "work", get measured as CPU burn much later, and are never
removed because by then everything depends on their timing. The
decision has to exist before the first protocol, or it is not a
decision.

## Approaches considered

**A. A sleep syscall** (`SYS_SLEEP(us)`). Simple and useless here: a
service that sleeps cannot serve. TCP must wait for "a segment
arrives OR the RTO expires", and a blocking sleep makes those
mutually exclusive unless every service grows a second thread and an
inter-thread protocol to go with it.

**B. A timer object with its own wait syscall.** Correct but
duplicative: a second thing to block on means every service's main
loop has to choose, and "wait for either" needs a new multiplexing
primitive — a `select` for a kernel that had happily avoided needing
one.

**C. Timers deliver a NOTIFICATION BADGE.** Chosen. Every service in
ArenaOS already parks in `SYS_WAIT` on exactly one notification whose
badge word is a set of merged bits: netd merges two MSI-X vectors
(ADR-0024), consoled merges two vectors plus the kernel's output-mirror
wake (ADR-0027), inputd merges an interrupt with its supervisor's
give-up word (ADR-0026). A timeout is simply one more bit. "Wait for a
device interrupt OR a client request OR a timeout" needs no new
primitive, no second thread, and cannot be missed by a service that
happened to be waiting on something else.

**D. Deadlines in the IPC call itself** (`call_with_timeout`).
Rejected as a special case of C that only helps IPC. The same
mechanism has to serve protocol timers anyway, and one facility that
covers both is smaller than two that overlap.

## Decision

Three syscalls:

- `SYS_CLOCK_NOW()` → monotonic microseconds. No capability: a clock
  reading is not authority over anything, and every timeout, RTT
  measurement and retry decision needs it. It is the same clock
  `timekeeping` calibrated in M2.2 and the m2 suite cross-checks on
  every boot, so a client and the kernel cannot disagree about how
  much time passed.
- `SYS_TIMER_ARM(notif_slot, badge, delay_us)` → timer id. Delivers
  `badge` on that notification once `delay_us` of monotonic time has
  passed.
- `SYS_TIMER_CANCEL(timer_id)`.

**The delay is relative, not absolute.** An absolute deadline makes
the caller read the clock, do arithmetic, and then race whatever
happens between the read and the call; "in 200 ms" cannot be stale by
construction.

**The capability gate is the notification.** Arming requires WRITE on
it — the notify side, exactly as `SYS_IRQ_RELAY` requires. A process
can only aim a timer at something it was already trusted to signal, so
timers needed no new capability kind to be safe. This is the third
time an existing capability has turned out to be the right gate for a
new mechanism (after `Power` for shutdown and `ConsoleInput` for
keystroke injection), and it is not a coincidence: if a new facility
needs a brand-new authority, that is usually a sign the facility is
doing two things.

**Resolution is stated, not implied.** Timers are checked on the
100 Hz tick, so a deadline means **not before**, with up to one tick
(~10 ms) of lag. That is ample for RTO minimums, ARP aging and DNS
retry. Nothing in Phase 7 may quietly assume finer; a caller that
needs to know how long actually passed asks the clock, which is
microsecond-true.

**One-shot only.** Periodic timers are re-armed by their owner. The
kernel side stays a fixed table with no drift policy to get wrong, and
a protocol that wants a repeating tick wants to choose the next
interval anyway — a back-off is not a period.

**Owned and swept.** Timers belong to the process that armed them and
are dropped in `proc::destroy`. That is now the fourth thing swept
there, after relay vectors (M5.2), the console output mirror (M6.4)
and blocked-thread references (M6.5): anything holding a pid gets
swept, as a rule rather than as four special cases.

**A tick dispatcher.** `kernel/src/tick.rs` owns the single auxiliary
tick hook and runs a fixed list of jobs. The console mirror had taken
that slot in M6.4, and "whoever registers last wins" is not a
mechanism. A tick task runs at interrupt context: it must not block or
allocate, and it may wake threads — which is the one privileged thing
a periodic job needs, and exactly what the IRQ relay path has done
since M5.2.

## Reasoning

The shape follows from a property the system already had. Because
every service blocks on one notification, a timer that notifies is
free to integrate: no service has to be restructured to use timeouts,
and none can be written in a way that accidentally cannot receive
them. Had timers been their own object with their own wait, every
Phase 7 service would have needed a multiplexing story before it
could have a timeout, and multiplexing is exactly where "just poll
for now" gets written.

## Downsides accepted

- **~10 ms granularity.** Fine for Phase 7, not fine forever. High
  resolution needs a programmable one-shot (LAPIC timer or HPET
  comparators) and a sorted deadline structure; both are additive and
  neither changes this ABI, which is the point of committing to
  "not before" rather than to a number.
- **A linear scan of 32 slots on the tick.** Trivial at this size and
  wrong at a thousand; a timer wheel or heap replaces it when a
  workload justifies one. The fast path already costs a single
  relaxed load when nothing is armed, which is every tick of every
  boot until something arms one — the lesson M6.4's console tap paid
  for in a failed clock calibration.
- **A fixed bound (`MAX_TIMERS`).** Arming can fail, and callers must
  handle it. Better than an allocation on a path the tick walks.
- **No timer inheritance across restart.** A supervised service that
  is restarted (ADR-0028) comes back with no timers, as it comes back
  with no other state. The stack must re-arm, which is the same
  re-establish rule already decided for netd's buffers.

## Future implications

- `STATUS_SERVICE_GONE` plus a timer is now enough for a client to
  bound ANY IPC call: arm, call, cancel on reply. ADR-0028's noted
  gap is closed in mechanism; making it a convenient library call is
  userspace's job.
- The netstack's main loop is now expressible: one notification,
  badges for device RX, device TX, client requests and every protocol
  timer. If a Phase 7 milestone ever finds itself spinning on
  `SYS_CLOCK_NOW`, the facility is wrong and gets fixed — polling is
  a named anti-goal of the phase, not a tradeoff.
- The tick dispatcher is where any future periodic kernel job goes
  (statistics sampling, watchdogs). It has three of four slots used;
  growing it is a constant, but a fifth customer is a good moment to
  ask whether that job really needs to run at interrupt context.
