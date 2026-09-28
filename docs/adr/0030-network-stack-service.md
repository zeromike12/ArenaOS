# ADR-0030 — ARP as a service: where the network stack ends and the driver begins

*Milestone 7.1. Status: accepted.*

## Problem

M6.1 gave ArenaOS a working NIC driver and proved it on the wire with
a hand-built ARP exchange. That proof was deliberately crude: the
*test* built the packet, because there was nowhere else for protocol
knowledge to live.

Phase 7 needs somewhere. The question this ADR answers is not "how do
we do ARP" — it is **where protocol state lives**, decided once,
before ARP, IPv4, UDP and TCP each answer it differently by accident.

## Approaches considered

**A. Protocols inside netd.** The smallest diff: netd already has the
frames. Rejected, and it is worth being precise about why, because
"keep the driver pure" on its own is aesthetics. The real reasons are
operational:

- A driver's job is to survive its device and be *restartable*
  (ADR-0028). A stack's job is to hold state across time. Putting
  them in one process means every ARP entry, and later every TCP
  connection, dies whenever the NIC wedges — and the supervisor's
  restart, which exists to make a dead driver survivable, becomes the
  thing that destroys all your connections.
- It makes the device the unit of protocol isolation. Two NICs would
  mean two stacks; a protocol bug would be fixable only by restarting
  the thing that owns the hardware.

**B. Protocols in the kernel.** Rejected on ADR-0002's terms: it is
not the smallest thing that must be privileged. A stack needs no
privileged instruction and no physical memory — it needs frames in
and frames out, which is an IPC contract.

**C. A separate userspace stack service.** Chosen. `netstackd`
(image 17) owns ARP and its cache; `netd` is unchanged and still has
never heard of an ethertype.

## Decision

**The split is structural, not documentary.** netstackd is granted no
device capability at all — an `Endpoint` to call netd with, an
`Endpoint` to be called on, and one frame. It could not touch the NIC
if it wanted to. netd, for its part, gained no protocol knowledge:
its only change is a *deadline* (below), which is about time, not
about packets.

**v1 is ARP over IPv4 and nothing else**: resolve an address by
putting a real request on the wire, cache the answer with a TTL, and
serve cached answers without touching the wire. That is the smallest
slice containing a real protocol, a real cache and a real failure
mode, and it exercises every part of the boundary the rest of Phase 7
will use.

**The deadline lives in netd, and that is the interesting part.**
`NET_OP_RECV` used to block until a frame arrived. For a *test* that
was fine; for a protocol it is fatal — one lost ARP reply parks the
stack forever. netstackd cannot bound its own wait, because a thread
blocked in `SYS_IPC_CALL` cannot observe its own timer (ADR-0029's
erratum, found in review). So the bound is passed **down**: RECV now
takes a timeout, and netd arms a timer on its own notification and
waits for "a frame arrived OR the deadline passed".

This is the first production use of the M7.0 facility, and it
vindicates building time before protocols: without it, the very first
protocol would have had a permanent hang in its unhappy path.

**Aging is by clock, not by timer.** Cache entries carry an expiry
and are checked when looked at. A timer per entry would buy nothing —
nothing has to *happen* when an ARP entry expires, and the only
moment staleness matters is the moment somebody asks. Timers are for
things that must act on their own.

**Retry policy follows the phase's decision 3.** An ARP request is a
broadcast query and therefore idempotent, so retrying it is safe and
netstackd retries three times before reporting `ARP_S_UNREACHABLE`.
That is *not* licence for the stack to retry everything: a datagram
send must never be re-sent after `STATUS_SERVICE_GONE`, because the
frame may already be on the wire. Today netstackd reports a dead
driver upward as `ARP_S_LINK_DOWN` rather than guessing.

## Reasoning

The honest test of a boundary is whether it can be crossed cheaply
and whether either side can be replaced. Here the stack talks to the
driver with the same IPC any client uses, holds no device authority,
and survives the driver being restarted under it *in principle* — the
proof of that is 7.1b, and it is named as not-yet-done rather than
implied.

The cache is proven the only way a cache can be: by showing the wire
stayed quiet. `arptest` reads the count of requests netstackd has
actually transmitted, resolves twice, and asserts the count did not
move. A cache that still sends the packet would pass any test that
only checked the returned value.

## Downsides accepted

- **One address family, one protocol.** No IPv4 forwarding, no ICMP,
  no fragmentation, no DHCP: the guest address (10.0.2.15) and
  gateway are hardcoded slirp facts, stated as fixture facts rather
  than dressed up as configuration.
- **A tiny cache with no eviction policy.** Eight entries, and a full
  cache overwrites slot 0. Honest and clearly marked; a real policy
  arrives when something needs one.
- **Frames that are not ARP are dropped.** v1 has nothing to give
  them to. This is fine today and becomes wrong the moment IPv4
  exists, at which point the receive path needs a demultiplexer —
  which is 7.2's first job, not a retrofit.
- **The stack itself is not yet supervised.** netd is (7.1b);
  netstackd is not, and should be once something depends on it.
- **Inline replies carry one payload word.** Returning a pair means
  packing, which is fine at this scale and will want a proper reply
  message shape before a protocol returns anything structured.

## M7.1b — surviving the driver

Decision 3 of the phase, done rather than promised.

netd is registered with the supervisor, killed mid-flight, and
restarted with its grant list replayed — including the same endpoint,
which is why netstackd's capability keeps working. What netstackd
does on `STATUS_SERVICE_GONE` is the part that matters:

- **It re-establishes, it does not retry.** A restarted netd is a new
  process that re-ran its own virtio handshake: fresh queues, freshly
  posted receive buffers, a fresh relay. Nothing it held survived, so
  the stack re-acquires its device facts (it asks the new instance
  for the MAC) and treats the success of that call as how it learns
  the driver is alive again.
- **It backs off on a real timer.** Spinning on `SYS_CLOCK_NOW` is
  the polling this phase forbids, and the stack cannot be woken by
  the supervisor — it has no way to know a spawn happened except by
  asking.
- **It re-sends only because ARP is idempotent.** A broadcast query
  costs a packet to repeat. A datagram send after `SERVICE_GONE` must
  never be repeated, because the frame may already be on the wire.

This is also where ADR-0028's amendment came from: netstackd's first
call into the corpse did not fail, it *queued*, because that ADR had
only covered calls already in flight.

**Making the test honest took three attempts**, and the failures are
the useful part. Restarting netd before releasing the client proved
only that a capability survives a restart — the stack called into a
driver that was already back and reported zero re-attaches. Polling
the supervisor inside the drain was no better: the first poll ran
before the client had even woken. The test now waits for the kernel's
own evidence that the stack has met the corpse — with netd dead, the
only thing left in the system that arms a timer is netstackd's
backoff — and only then lets the supervisor work. A proof that cannot
fail is not a proof.

## Future implications

- The receive path is the seam everything else hangs from. When IPv4
  lands, `netstackd` grows a demultiplexer and ARP becomes one
  consumer of it rather than the only one; the RECV-with-deadline
  contract stays as it is.
- netd's deadline argument is the template for every future driver
  call that waits: the server enforces the bound because the client
  cannot. That is now a rule of this codebase, not a local trick.
- A second NIC would mean a second netd and one netstackd with two
  L2 endpoints — which is the arrangement approach A could not have
  expressed at all.
