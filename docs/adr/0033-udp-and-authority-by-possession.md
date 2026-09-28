# ADR-0033 — UDP, and a port handle that is authority rather than identity

*Milestone 7.4. Status: accepted.*

## Problem

UDP needs a way to say who may use a port. The phase plan called this
"a capability question, not a protocol one", which is right, and I
then drew the wrong conclusion from it twice in a row — once toward
the kernel, once toward caller identity. Both are worth recording,
because the correct answer is the one this system already uses
everywhere else and I walked past it.

## What I got wrong, and the correction

**First instinct: a kernel capability.** A `CapObj::UdpPort` minted by
the kernel. Rejected, and still rejected: it requires the kernel to
know what UDP is, which ADR-0030 spent a milestone keeping out of it,
and the kernel cannot enforce anything about a namespace it cannot
see. That part of ADR-0032's appendix stands.

**Second instinct: per-client endpoints.** Having established that
`ipc::recv` reveals nothing about the caller, I concluded that
per-binding authority therefore required one endpoint per client.

That was wrong, and C's review said so: **a shared endpoint can hand
back an opaque unforgeable token at BIND, and possession of the token
is the authority.** No pid, no caller identity, no kernel involvement.

The mistake was conflating **authority** with **identity**. ArenaOS
answers "may you do this?" with "do you hold the reference?"
everywhere — `Power` for shutdown, `ConsoleInput` for keystrokes, an
`Endpoint` for a service, a notification's WRITE right for a timer.
A service-issued token is precisely that idea one layer down, and
reaching for identity was the un-ArenaOS instinct.

What remains true is narrower and still useful: **non-transferable
per-process ownership** is not available on a shared endpoint under
IPC v1. If a policy needs "this process and no other", it needs
caller identity, which the ABI does not provide. That is a real
limit, and it is not the limit I originally claimed.

## Decision

`BIND(port)` returns a **64-bit handle drawn from rngd**, and
`SEND`/`RECV`/`CLOSE` take that handle. Possession is the authority;
deliberately passing the handle to another program is delegation.

**The handles are random on purpose.** A handle is authority by
possession, so a guessable handle is authority by arithmetic — an
index would have been security theatre. If entropy is unavailable
netstackd **refuses to bind** rather than issuing a predictable
token: an obvious refusal is better than a false assurance, and it
keeps the failure in the place where it can be seen.

**Bind-once is a separate rule.** A port may be bound once; a second
bind is refused. That is a namespace question, not an authority
question, and the two are tested separately — a second bind is
refused, and a forged handle buys nothing.

The demultiplexer (ADR-0031) grew one arm; ARP, ICMP and UDP now
share it, which is what that structure was for.

## Proof

A real DNS query for `example.com` to slirp's resolver at
10.0.2.3:53, with the response matched on **our** transaction id and
the response bit — a datagram that merely arrived would not pass.
Plus a second bind refused, and a forged handle refused.

Using a real server rather than a loopback matters: it means the
datagram was well-formed enough for software that did not come from
this repository to parse and answer it.

## Downsides accepted

- **The UDP checksum is sent as zero** (legal in IPv4) and a nonzero
  incoming one is **not verified**. Said out loud rather than left to
  look like an oversight; it is the next thing to fix in this layer
  and it should arrive with a synthetic frame in the parser
  self-test, per ADR-0031's template.
- **A binding's inbox is single-slot.** The newest datagram replaces
  the previous one. UDP is allowed to drop, and a queue nobody drains
  is a leak that looks like a feature; a real receive queue is a
  design with back-pressure, not a bigger array.
- **RECV delivers the first message-worth of a datagram**, with the
  full length reported so the caller knows what it did not get. The
  chunked pattern from ADR-0032 applies here too and is not built
  yet.
- **No per-process ownership**, as above: possession is the whole
  story, and a program that leaks its handle has delegated.
- **Eight bindings**, matching the handle pool drawn at startup.

## Future implications

- DNS (7.5) is now a parsing exercise rather than a transport one,
  and will need the rest of a datagram — so the UDP receive path gets
  the same chunked treatment the frame path got.
- If a future policy genuinely needs identity rather than authority
  — "only THIS process may rebind" — that needs either a caller
  identity word in the IPC ABI or per-client endpoints, and it should
  be driven by a real requirement rather than by symmetry with other
  systems.
- C's longer-term suggestion of a generic service-defined opaque
  kernel capability would let a service hand out authority the kernel
  can carry without the kernel learning the protocol. Worth its own
  ADR when a second service wants the same shape; inventing it for
  one service would be building a subsystem to avoid a 20-line token
  table.
