# ADR-0055 — Capability-gated, revocable dynamic Image objects

Status: **Proposed — literal ABI/lifetime proposal for C's review; nothing below exists yet**
Date: 2026-09-30
Milestone: Phase 8.5 kernel mechanism. [ADR-0054](0054-installed-images-and-atomic-upgrade.md) owns the distinct persistent install/activation transaction. This ADR does not accept that transaction or authorize installer code.

## Boundary and measured input

The existing Image kind names only kernel-embedded IDs **0..26**. A staged APKG is not
an Image. C selected a ring-3 verified snapshot handed to a narrow capability-gated
kernel copy/validation primitive. Ring 0 must not parse AFS1/APKG/APOL or verify
signatures; the registrar bearer is the execution-verification TCB. Neither caller
name/pid/path nor an old `ELIGIBLE` reply establishes verification. The kernel does not
know that an ELF is signed.

A genuine static `ET_EXEC` with cap-mediated package QUERY behavior builds twice
byte-identically to **648 bytes**, passes the *unmodified* production `elf::validate` in
a host adapter, and fits a separately host-signed/verified **840-byte APKG v1**. See
[evidence](0054-elf-v1-evidence.md). This is **not guest execution**. Keep v1's
4096-byte payload ceiling for this narrow proof; larger general applications may require
a *separate*, explicitly versioned signed-wire decision, not an unmeasured v2 now.

The following is a **candidate exact contract**, additive to syscall ABI v1. Reviewing
it is not accepting it.

## Cap inventory and transfer topology (proposed)

New `CapObj::ImageRegistrar` is a single kernel-created boot object. Only `WRITE`
authorizes `REGISTER` and `REVOKE`; object possession, not pid, grants access. The
kernel records the boot manager's pid solely for the separate *lifecycle* fail-stop
hook; that pid is not an authorization check on these syscalls. `COPY` is required only
to delegate, `DESTROY` to discard a locally minted/inherited cap. No ordinary caller can
mint a registrar; `SYS_CAP_COPY` only attenuates. The protected manager boot anchor is
`WRITE|COPY|DESTROY`; packaged (the existing trusted ring-3 verifier, not a newly
privileged app) inherits `WRITE|DESTROY`, **without COPY**. No fsd, shell, broker,
ordinary app, or dynamic child receives it. The manager must never pass a registrar as a
dynamic child's inherited grant.

Use the **existing** packaged endpoint, not a speculative private eleventh endpoint:
current `ipc::MAX_ENDPOINTS=10` is filled by ten boot-created endpoints. A separate
newly created Notification object, distinct from the 8.4 STAGE approval object, is the
receiver-checked 8.5 manager-approval marker. **Notification capacity is also currently
full (17/17)**; this proposal explicitly requires a reviewed increase to **18 fixed
entries** solely for this one disjoint marker, with a boot inventory proving 18/18 and
the nineteenth creation refusing without mutation. This is an additive
resource-bound/boot-policy decision, **not** an implemented or automatically approved
table change. The manager holds a `READ|COPY|DESTROY` source; packaged holds a `READ`
anchor and verifies *object ID plus exact sent rights* (`READ|COPY|DESTROY`) on every
INSTALL, SELECT prepare/commit, DEACTIVATE and privileged LAUNCH/return-Image operation.
The received IPC-landed marker is discarded **on all paths**, including malformed calls
and PING; an integer slot, endpoint/WRITE or stale STAGE marker alone never approves.
This marker is manager-only, **not** silently granted to shell. Normal unprivileged
QUERY remains marker-free. Whether INSTALL and SELECT need *separate* new markers is a
trust-policy acceptance gate in ADR-0054; if yes, the current five-grant budget cannot
be pretended away.

The current packaged inherited grants are fsd Endpoint/WRITE (0), package Endpoint/READ
(1), STAGE Notification/READ (2). Candidate additional registrar (3) and new manager
marker anchor (4) total **5**, exactly `spawn::MAX_INHERIT=5` and manager
`MAX_GRANTS=5`. The manager's audited 20 literal boot caps would become 22 (registrar
and new marker), below 32; the actual dynamic Process-cap and temporary cap-slot
high-water mark still need an inventory test. No new IPC endpoint, extra inherited
grant, marker alias with STAGE, or implicit increase of either five-cap limit is
authorized. The same endpoint is a *transport only*: packaged may transfer an Image back
**only** in a manager-marker-approved call whose operation, signed digest, version and
commit result it has independently checked. Manager validates the reply status, expected
image ID/kind/rights and full-file digest; unexpected reply caps must be destroyed, not
ignored. A single marker is not a substitute for fresh signer/current-policy checks or
an installer request state machine.

Existing IPC snapshots a send-cap **by value**, leaves the sender's slot intact, and
transfers the entire rights mask to the receiver; it does not attenuate at reply time.
To reply with an Image, the verifier's registered source has `READ|COPY|DESTROY`, and
`SYS_IPC_REPLY` stages that exact cap. On successful reply the manager receives an
IPC-landed `READ|COPY|DESTROY` reference; it `SYS_CAP_COPY`s an attenuated
`READ|DESTROY` cap into a free slot and destroys the landed transitional cap. The
verifier destroys its original after the completed transfer or explicit abort. The
manager alone keeps the final `READ|DESTROY` reference: it can spawn and clean up but
cannot pass it to a child or third party. Destroying either reference does **not**
revoke other copies. If the reply is lost, the verifier dies, or the caller is killed,
the reference rules below retire the provisional object when its last actual reference
disappears. A durable activation never relies on an unacknowledged in-memory cap;
explicit LAUNCH can freshly reverify and register. Any other service receiving an
unexpected transferred Image must destroy it; no new app gets a registrar or a
transferable Image.

## Additive syscall ABI v1 (candidate numbers 33 and 34)

Use the existing x86-64 ABI: RAX number; RDI, RSI, RDX, R10, R8, R9 arguments in order;
RAX signed `i64` result. All unused argument registers for each new call must be zero
(including R8/R9). Unknown flags are not silently accepted. Both calls are nonblocking
and execute in the caller's address space under the existing IF=0 syscall discipline.
`BAD_ARG=-2`, `BAD_ADDRESS=-3`, `BUSY=-4`; no new error number is proposed. All success
results are positive for REGISTER and zero for REVOKE.

| Call | Exact inputs | Success | Refusal (no cap/object/ID mint on REGISTER refusal) |
|---|---|---|---|
| `SYS_IMAGE_REGISTER=33` | `RDI` registrar cap slot (0..31, `ImageRegistrar/WRITE`); `RSI` user virtual address; `RDX` exact ELF byte length (1..=4096); `R10` **empty** destination slot (0..31); `R8=R9=0` | Kernel copies exactly `len` once into reserved kernel-owned bytes, validates *that copy* with unchanged `elf::validate` and the dynamic page budget below, mints `Image {fresh_id}/READ|COPY|DESTROY` into destination, returns `fresh_id` as positive `i64`. | Invalid arg, registrar/rights, slot, occupied destination, unused register or ELF: `BAD_ARG`; overflowing/unmapped/not-user-readable span: `BAD_ADDRESS`; two slots busy or ID space exhausted: `BUSY`. No partial published state on error. |
| `SYS_IMAGE_REVOKE=34` | `RDI` registrar cap slot (`WRITE`); `RSI` **full** dynamic `u32` image ID encoded as `u64`; `RDX=R10=R8=R9=0` | Atomically sets matching live ID `REVOKED` for every cap, retires bytes when all in-progress loader pins are gone, returns 0. | Wrong cap/rights, embedded 0..26, out-of-range ID, unknown or already stale ID, nonzero reserved args: `BAD_ARG`. Never uses numeric ID alone as authority. |

Validate arg/rights/destination and bound/reserve before touching user bytes. For
REGISTER, validate `[addr,addr+len)` with checked addition and **each page** against the
caller's readable registered regions; reject zero/holes/kernel-half or overflowing
spans. Copy under a paired STAC/CLAC into fixed kernel storage, not a 4096-byte syscall
stack array; caller edits/unmaps after return cannot change it. On a copy fault the
syscall must refuse without publishing (do not rely on a recoverable kernel #PF that
does not exist). The already-copied length, not unused storage tail, is passed to
production `elf::validate`. Require the ELF's parsed entry/segments/stack address to be
loadable under the existing `spawn::prepare` arithmetic: checked top/rounding, one page
for stack strictly in the user half, and region count including stack. Do **not**
replace or weaken the existing ELF validator. Cache no signer/policy assertion in the
kernel.

`SYS_CAP_DESCRIBE` remains `[kind, object_id, rights]`; propose ImageRegistrar **kind
5**, object ID **0** (descriptive, not an invoke token). Existing live Image remains
kind **1** with the full monotonic ID. Describe a **stale** dynamic Image as `BAD_ARG`,
without writing the output buffer; READ permission alone on Image still gates SPAWN.
Existing `SYS_CAP_COPY` may copy a stale reference (its rights still attenuate), and IPC
may carry one, but neither operation can *reactivate* it. `SYS_CAP_DESTROY` on a locally
minted Image needs `DESTROY`; IPC-landed copies additionally retain the existing
ADR-0047 discard provenance. Source/destination bounds, exact rights and status for
existing syscalls stay unchanged.

## Registry identity, page budget and spawn linearization

Two fixed slots, each **4096 bytes** plus bounded metadata, can represent current and
proposed replacement simultaneously; the slot index is **not** object identity.
`next_id: u64` starts at 27 at boot, increments *only on successful mint* and never
decreases within a boot (including revoke, failure and physical slot reuse). Mint only
while `next_id<=u32::MAX`; after successfully issuing `u32::MAX`, hold `u32::MAX+1` and
return BUSY forever. Embedded IDs 0..26 and their existing static match are unchanged;
`spawn::MAX_IMAGES=24` is stale even for the static 0..26 match, **not** a dynamic
registry bound. Never allocate a new embedded ID for this fixture.

Additional dynamic admission bound: sum over validated `PT_LOAD` segments of
`(checked_ceil_4k(vaddr+memsz)-vaddr)/4096`, with all sums checked, must be **<=16
pages**. Include the *separate* one-page stack in spawn/resource preflight (so <=17 user
leaf pages plus bounded page tables). Refuse invalid rounding, stack overlap, stack
beyond user half or exceeding `sched::USER_REGIONS_MAX` **before mint**. Existing
`elf::validate` allows large in-range `memsz`; this bound prevents a 648-byte signed
file from forcing an effectively unbounded page allocation. This 16-page dynamic limit
does not change static embedded-image acceptance.

Object state: `FREE -> RESERVED (unpublished) -> LIVE(id,len,bytes,refs,pins) ->
REVOKED/RETIRING -> FREE`. Last-reference retirement also enters RETIRING immediately,
so no new spawn is allowed while a loader pin drains. Storage is freed only after
`pins=0`. On any REGISTER refusal erase the reservation, do not consume an ID, and
leave the destination empty. Mint, ref increment,
publish `LIVE`, and return ID in a single IF=0 commit step; if mint fails, unwind the
reservation and ID. Keep the copy immutable and owned by kernel memory, never by
caller/AFS1/LENT frame. Explicit REVOKE changes liveness at its linearization point:
every copied Image ref to `id` becomes unusable *immediately*, even if it still occupies
a slot. A recycled slot has another ID. Actual cap removal is not required to invalidate
old authority. `REVOKED` bytes can be erased/reused only after pins hit zero; old stale
cap refs may remain, but ref release for that ID must not affect a new slot occupant.
Treat ref/pin overflow, underflow and impossible duplicate mutation as a fatal kernel
invariant, not success or silent wraparound.

On **every** `SYS_SPAWN`, first read the held Image/READ cap, resolve the full ID under
IF=0 and check `LIVE` before reserving any child/record/frame/Process cap. Pin the
registry object across `spawn::prepare`, its second `elf::validate`, and `elf::load`
until all image bytes have been copied into child-owned frames; unpin on *all* exits and
rollback paths. Calls are serialized under the one-core IF=0 discipline for this
proposal: a blocking/preemptible operation must never retain a naked borrow of the
registry table. A concurrent revoke cannot cross the pin without an explicit reviewed
locking change. After load, child pages belong to its address space and revoke affects
**future** spawns only. Source mutability, dynamic-ID slot aliasing and stale-cap/revoke
races must be tested at this exact linearization point. If supporting
reentrant/multicore spawn later, revisit pin synchronization in a separate decision.

## Reference ledger: every transition must balance

An issued Image cap in **each** process slot counts one `refs`; so does **each**
separately staged Image copy in an IPC queue `send_cap` or `reply_cap`. A local `Cap`
copied from an already-counted slot as a short IF=0 validation temporary is not an
additional ref. During an IPC queue-to-slot transfer, however, its one count remains in
**escrow** until a landing slot is committed or the transfer drops it: current
`take_request`/reply-resume code clears the queue field *before* `cap::grant`, so simply
releasing the staged count at field clear would prematurely retire the object. Never
schedule/block with an escrow ref or leave one after syscall completion. A transient
loader borrow counts one `pins`, **not refs**. Bounds: `proc::MAX_PROCESSES=32` x
`cap::CAP_SLOTS=32`, plus `ipc::MAX_ENDPOINTS=10` x `QUEUE_DEPTH=4` x at most two staged
cap fields, make counts finite; checked arithmetic still required. Ref counting is
**not** the existing cap implementation's behavior; this is a required additive, audited
mutation hook, not a free consequence of `Cap: Copy`.

| Transition | Required ledger action |
|---|---|
| REGISTER success / failed mint | Exactly one ref for the destination on success; no ref, cap, live slot or ID advance on refusal. |
| `SYS_CAP_COPY`, inherited Image at SPAWN, kernel `grant`/`issue` and `move_cap` | Increment on newly published copy; on move transfer the *same* count, never increment-then-retire through a zero-ref gap. Occupied destination and failed child creation roll back increment. Static images have no dynamic ref ledger. |
| `SYS_CAP_DESTROY`, `consume`, any overwriting `install`/root grant | Decrement the displaced dynamic cap **once** only after the replacement/removal is committed. Destroying one cap is *not* revoke. No silent overwrite of a live cap, including `ipc_landed` provenance. |
| `SYS_IPC_CALL` / `SYS_IPC_REPLY` enqueue | Snapshot the send cap (current `send_cap_of` does not attenuate), add one count **only after** a queue slot is committed; a refused/orphaned/full call or reply adds none. Sender's slot remains separately counted. |
| IPC request delivery / reply resume | Move the staged count into a successfully landed slot **without a gap**; if recipient table is full or dead, drop staged count. A `REPLIED` cap remains counted even when its sending server dies; landing later succeeds or drops, then decrements as needed. An already-landed cap is owned/countable by the recipient capspace, **not** the queue. |
| Server death, caller death / late reply | `fail_calls_for_server` turns Waiting/Delivered to Failed and drops any still staged request cap; a Replied slot remains for the caller. `release_blocked_of` clears Waiting/Delivered/Replied/Failed queue slots, dropping each still-staged cap (including an unanswered late reply) exactly once. A subsequent server REPLY to an abandoned call is BAD_ARG and publishes no ref. The server's already-landed request cap is released by its process sweep, not twice by the queue. |
| `proc::destroy` and `spawn::rollback` | After IPC teardown, walk all 32 slots of the dying process and decrement dynamic Image refs before `procs[idx]=None`; avoid aliases of a borrowed `PROCESSES` table/IPC table. On last ref of LIVE, mark dead and retire storage (after pins) without requiring a registrar. Fail-stop manager check below precedes its capspace loss. |
| REVOKE and later stale-cap deletion | Mark ID dead atomically independent of `refs`; preserve an ID-keyed tombstone count while stale copies exist, or ensure equivalent generation-safe accounting. Release storage once pins=0; dropping stale refs after reuse must not decrement the replacement slot. Never allow ref release to make a revoked object LIVE. |

The ledger has one required conservation invariant per LIVE dynamic ID at stable syscall
boundaries: `refs = number of published process slots + queued send_cap + queued
reply_cap` (plus a strictly IF=0 transfer escrow during the internal move only). A
`LIVE` object with `refs=0` retires automatically. For a revoked ID, implement an
ID-keyed bounded tombstone or prove that discarding stale refs without a retained count
preserves all accounting and does not reuse ID metadata unsafely. Because IDs never
repeat and `refs` is bounded, a safe alternative is to *not* count stale refs after
revoke, so REVOKE drops all object-reference accounting while the cap slots retain only
inert numeric IDs; subsequent slot clear skips ref decrement after ID lookup misses.
This alternative is the **selected candidate**: the conservation equation applies to
LIVE IDs, tombstones are unnecessary; stale caps can never target new IDs. Before
freeing a revoked object's bytes, require pins=0. A failed `cap::grant` in IPC cannot be
treated as a landed cap for accounting.

## Manager death and dynamic-child ownership (fail-stop candidate)

Current `SYS_SPAWN` hands its caller Process/READ|DESTROY (no COPY), and
`SYS_PROC_FINISH` consumes that held cap to stop/reap the child. A different userspace
service cannot reacquire it by pid or title. `Image` revoke does **not** stop a running
child. No general process-ownership/reacquisition syscall is proposed.

At boot record the *specific kernel-spawned manager root*'s pid for lifecycle detection
only, not for registration authorization. Tag successful dynamic `SYS_SPAWN` user-child
records with their full dynamic image ID; tag is written before starting the first
thread and cleared only on successful `SYS_PROC_FINISH`/record forget (rollback clears a
half-built record). Check for **any unretired dynamic child record**, including an
already-exited but not yet reaped child, and for **any LIVE dynamic registration**. If
that protected manager reaches its *last live thread* via `SYS_THREAD_EXIT` or CPL3
fault, **or** its address space is about to be destroyed from any kernel path, and
either condition holds, call `halt::halt_machine("phase85: manager lost with dynamic
image/child")` **before** `proc::destroy`'s existing `fail_calls_for_server` IPC sweep,
removing caps, clearing threads, freeing frames or accepting another spawn. This is intentionally stricter than just a currently executing child and
avoids a last-thread-exit zombie with a non-copyable Process cap. The manager check must
not trigger on ordinary supervised service death; an orphaned endpoint/restart is not a
way to obtain child teardown authority. The same last-thread/pre-destroy hook marks the
registrar singleton **DEAD** on every manager death, invalidating all its copies, so
even a still-running verifier cannot register after the manager is gone. The liveness
change is not a caller identity check: ordinary registration still tests the *held*
registrar cap plus singleton liveness. If manager dies when *no* dynamic registration or
unretired dynamic child exists, the registrar stays dead until a fresh approved boot
establishes authority; no implicit manager resurrection/boot launch. If a LIVE dynamic
registration exists, the hook halts before normal teardown, even if no child has yet
launched. A stuck-but-live manager is a separate liveness failure and must not be called
recovered.

Place the last-thread check at the start of both `sys_thread_exit` and
`exit_on_user_fault`, before `record_exit`, `notify_last_thread_exit` or
`sched::terminate`; place the pre-destroy check at the start of
`proc::destroy`, before its IPC sweep. `SYS_PROC_FINISH` already refuses
kernel-bootstrapped manager targets; this hook also covers internal destroy.
One-core IF=0 ordering means a committed spawn record cannot slip between the
manager-death check and halt. If a proposed implementation can miss a manager death
route (kernel-driven destroy, return from fault, thread exit, last-thread scheduling),
fail-stop is **not** proved and dynamic-child execution must not ship. A fatal halt is a
deliberate fail-closed availability cost, not a QEMU PASS or an anti-rollback guarantee.
Only an accepted, separately justified general ownership/reacquisition design can
replace this rule.

## Explicitly out of scope and acceptance gate

No production-key ceremony, Secure Boot, hostile-disk rollback guarantee, dynamic
linking, GC, multi-package namespace, kernel fsd/crypto, ambient pid-based register
call, implicit boot launch or user-space ELF loader duplication. Existing embedded
images and Phase 8.4 staging remain unchanged. Reboot discards registry/caps/IDs and
creates no dynamic Image until ADR-0054's complete durable rescan and fresh exact
signed-file/current-policy verification on an explicit request.

For C's acceptance, audit actual boot grants and manager/package high-water cap
inventory, marker authority and exact request/reply protocol; freeze registrar
kind/rights, number/status encodings, scratch-memory/page-budget rules, reference hooks
including every IPC state and capspace sweep, spawn/revoke linearization, and
manager-death path. If one 8.5 marker cannot legitimately authorize both INSTALL and
SELECT, or the five-grant limit/32-slot capspaces cannot support the chosen flow, return
to design review; do **not** add an unreviewed endpoint, expand grants or fake a private
channel. ADR-0054 must independently freeze its persistent wire, crash-prefix and
capacity rules. Neither ADR is Accepted yet.

Mandatory host tests: two-slot refusal, ID 27..u32::MAX without wrap (model near
exhaustion), stale-slot reuse, malformed/cross-page/mutated caller bytes, huge
memsz/stack overflow, failure injection for every grant/copy/drop/mint and conservation
after all Waiting/Delivered/Replied/Failed transitions. Mandatory guest tests: registrar
bearer vs wrong/right-stripped cap, actual two different signed payloads, successful
same-path `SYS_SPAWN` + real strict child IPC/exit, attenuation/marker inventory, copied
stale Image refusal before/after slot reuse, provisional verifier death with automatic
retirement, stalled/lost replies and server/caller teardown,
manager-death-with-live-child terminal diagnostic, and historical 8.4 regressions. Full
suite, fresh final-artifact-bound 100/100 QEMU and extracted deployable boot remain
milestone gates, **not** evidence already achieved here. Phase 8.4 stays the last
qualified checkpoint.
