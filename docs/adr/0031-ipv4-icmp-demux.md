# ADR-0031 — IPv4 and ICMP: the receive demultiplexer, and how little IPv4 to build

*Milestone 7.2. Status: accepted.*

## Problem

ADR-0030 shipped one protocol and named the simplification it was
leaving behind:

> Frames that are not ARP are dropped. v1 has nothing to give them
> to. This is fine today and becomes wrong the moment IPv4 exists, at
> which point the receive path needs a demultiplexer — which is 7.2's
> first job, not a retrofit.

That moment is now. There are two questions: **who recognises a
frame**, and **how much IPv4 is the smallest honest amount**.

## The demultiplexer

With one protocol, `resolve` read frames itself: call netd's RECV,
check whether the frame is the ARP reply it is waiting for, discard
otherwise. That is not merely inelegant with two protocols, it is
**wrong**: an echo reply arriving while the stack happens to be
resolving would be discarded, and the ping waiting for it would then
time out — a fault with no cause anywhere near where it appeared.

So recognising a frame is one job in one place:

- `pump` takes exactly one frame from the driver (with netd's
  deadline, ADR-0030) and hands it to `demux`.
- `demux` dispatches on ethertype: ARP, IPv4, or counted-and-dropped.
- Operations no longer read the wire. They set what they are waiting
  for, call `pump`, and watch state the demultiplexer updates.

The counters are part of the contract, not decoration: frames seen,
per protocol, dropped, and failed-checksum. **A stack that cannot say
what it dropped is not demultiplexing, it is guessing** — and the
test asserts it saw both protocols, because with one consumer there
is nothing to prove.

## How much IPv4

Deliberately the smallest thing that deserves the name: a fixed
20-byte header, no options, no fragmentation, and no routing — every
destination is treated as on-link and resolved with ARP. TTL is 64
because that is what everything else picks.

What IS implemented is implemented properly, and that distinction is
the whole point of the scope:

- **Both checksums are computed on send and VERIFIED on receive.**
  A frame that fails is counted and dropped, never trusted. The wire
  is the one input a stack must not take on faith, and a stack that
  skips the check works perfectly until the day it matters.
- **A packet must be addressed to us** and carry ICMP to get further.
  There is no forwarding: this is a host, and says so.
- **An echo reply must match the identifier AND the sequence** that
  went out. A reply that merely arrived proves nothing — it could be
  an answer to somebody else's ping.
- **Echo requests are NOT answered.** Nothing has asked ArenaOS to be
  pingable, and slirp will not send an unsolicited echo, so a reply
  path would be untested code shipped on the strength of looking
  right. Twenty lines is not worth an untested protocol path.

## Reasoning

The layering has to be visible in the FAILURES, not just the happy
path, or it is not really layered. A ping to an unresolvable address
fails `UNREACHABLE` — there is no host to send an echo to, and the
failure happens at ARP. A resolved host that stays silent fails
`NO_REPLY`. A diagnostic tool that cannot tell those apart is not a
diagnostic, and the fact that netstackd reports them distinctly is
evidence that IP really is sitting on ARP rather than tangled with it.

Timing the round trip with `SYS_CLOCK_NOW` rather than counting
retries is the same instinct as everywhere else in this project:
report the measurement, not the inference.

## Downsides accepted

- **No fragmentation and no options.** A frame larger than netd's
  64-byte inline reply limit cannot even be received today, so
  fragmentation would be theatre before that limit is lifted.
- **No routing table.** Every address is assumed on-link. Correct on
  the slirp network ArenaOS boots into and wrong on any real one; a
  routing table arrives with the first milestone that has somewhere
  to route to.
- **No DHCP.** The guest address is a hardcoded slirp fact
  (10.0.2.15), stated as a fixture fact rather than dressed up as
  configuration.
- **Not pingable.** See above; deliberate.
- **The receive path is still one frame per IPC call.** Fine for ARP
  and ping, and the first thing UDP throughput will complain about.

## Future implications

- UDP is now a small step: the demultiplexer grows one arm, and the
  interesting design question moves to **ports** — who is allowed to
  bind one, which is a capability question rather than a protocol
  one, and deserves its own ADR rather than an afternoon's choice.
- netd's 64-byte inline reply limit is the next real constraint. A
  DNS response or a TCP segment will not fit, so the frame path will
  need the lent-buffer treatment the SEND path already has — receive
  into a caller's own frame rather than through the message.
- The checksum routine is shared by IP and ICMP and will be shared by
  UDP and TCP, where it also covers a pseudo-header. Worth keeping in
  one place as that arrives.
