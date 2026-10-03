# ADR-0064 — Bounded multiple dynamic children and reply cancellation

Status: engineering decision implemented; wider qualification pending.

Replace ADR-0055's one-unretired-dynamic-child qualification bound with **four**
structurally tagged dynamic spawn records system-wide. The desktop's separate
six-session window bound includes static BootImage children; the installed
4096-byte dynamic-image mechanism supports up to four concurrent children.
Two live registry Image IDs can supply multiple independent children each.
Neither storage slot count nor a name/package ID grants execution authority.

The useful bound follows the real manager working set: baseline 25 observed
caps, 27 with an unretired child, 28 with two Image IDs. Filling the other three
child entries reaches 31, retaining an IPC/temporary-cap slot in the 32-slot
space. The guest capacity probe starts three further ordinary SYS_SPAWN
children, lets their ring-3 programs actually run, observes their held Process
liveness, refuses the fifth and compares exact own-cap occupancy and actual
process/thread-table rows across refusal. It stops/reaps every extra child and
returns to the original cap count. It is invoked while the original child is
live and while it is exited/unreaped, and through repeated STOP/FINISH cycles.
Independent Power-shell frame/process/record snapshots continue to check exact
restoration around the historical live cutover. Full desktop peak measurement
and cross-parent graphical dynamic-Image proof remain qualification work.

Both syscall preflight and the IF=0 spawn-record reservation scan enforce the
finite bound. An exited child still counts until successful Process-cap FINISH
forgets its record. Existing rollback, immutable loader pin, image reference
ledger, revocation and monotonic full-ID checks are retained. Revocation blocks
future spawns without invalidating already copied process pages. Manager-death
fail-stop still checks **any** unretired dynamic child, not capacity fullness.
Historical single-child BUSY assertions are explicitly superseded by the
stronger fill/refuse/live/exited probes; the old cutover/selection/resource
fixtures remain and must pass. No trust identity or persistent format changes.

The initial four-child guest probe exposed a cancellation bug: killing several
callers while packaged serviced their queued filesystem-backed queries caused
its next reply to be BAD_ARG, treated as server failure. Add checked reply
`SYS_IPC_REPLY_CHECKED=43`, with the same Endpoint/READ gate and buffer/cap
validation as legacy REPLY. It consumes a Cancelled tombstone for this exact
server thread and returns CALLER_GONE=-6 without staging reply references.
Missing requests and other misuse still fail BAD_ARG. Legacy REPLY semantics
remain unchanged for historical callers and cancellation fixtures.

Packaged handles CALLER_GONE as a departed caller. It revokes/drops any unsent
provisional Image before continuing to receive work. It retains fail-stop for
other reply errors. No caller PID is supplied or trusted; the IPC queue's
existing delivery/cancellation ownership identifies the request. Mutation
controls must prove omission of cancellation handling fails the multi-child
workflow and restoration succeeds. Multi-child quota and stale/ownership
controls are required before engineering-baseline qualification.
