# ADR-0032 — Frames larger than one IPC message, and why this came before UDP

*Milestone 7.3. Status: accepted.*

## Problem

`netd` dropped any received frame longer than one IPC inline message
(64 bytes) and answered `NET_S_BAD_LEN`. ADR-0024 called that "the
documented v1 limit", which was honest, and ADR-0031 named it as the
next real constraint.

It is worse than a limit: it silently decides what protocols can
exist. ARP replies are 42 bytes and small ICMP echoes fit, so
everything through M7.2 worked. A DNS answer does not fit. A TCP
segment does not fit. Phase 7 could not continue past ICMP.

## A reordering, stated rather than done quietly

The plan after 7.2 said UDP next, with the frame-size limit "named as
the next constraint". Doing UDP first would have meant building a
protocol that could only carry datagrams smaller than 64 bytes, then
immediately reworking its receive path — and writing a UDP test that
passed for reasons the real world would not reproduce.

So the receive path came first, as its own milestone, and UDP
inherits a frame path that already works. Smallest useful step, in
the order that makes each step provable.

## Decision

`NET_OP_RECV` now **stages** the frame instead of copying-or-dropping
it: the frame stays in the receive-ring buffer where the device put
it, the reply carries the frame's **full length** plus the first
chunk, and `NET_OP_RECV_CHUNK(offset)` reads the rest. The buffer
returns to the ring when the last chunk is taken.

Consequences worth being explicit about:

- **This is not zero-copy and does not pretend to be.** It costs one
  IPC round trip per 64 bytes: right for a DNS answer, wrong for
  throughput. The zero-copy path — the caller's own frame posted
  into the receive ring so the device DMAs straight into it — is a
  larger change touching ring ownership, and it gets its own
  milestone rather than being half-built here.
- **Oversize frames still drain.** A frame longer than the stack will
  reassemble is counted and discarded, but every chunk is still read,
  because the driver only releases its buffer on the last one.
  Abandoning it would leak a receive buffer per oversize frame — the
  kind of leak that only shows up under the traffic that causes it.
- **The stack's reassembly bound is stated** (`FRAME_MAX`, 512) rather
  than implied by whatever buffer happened to be in scope.

## Proving it, by making it unavoidable

The temptation was a test that sends one big frame. Instead the
**existing** proof was made to require the new path: the ICMP echo
payload went from 8 bytes to 200, so every boot now puts a 242-byte
frame on the wire and gets one back. There is no configuration in
which the old code passes.

The payload is verified byte-for-byte, and the pattern is
**position-dependent** (`b"arenaos!"[i % 8] ^ i`). Constant bytes
would let a reassembly that dropped, duplicated or reordered a chunk
pass unnoticed; matching identifier and sequence certainly would.
First boot after the change: four chunks, 200/200 bytes intact.

Two bugs were caught by those first boots rather than by reading the
code: the payload filler indexed an 8-byte literal 200 times (a clean
panic), and the parser self-test's control frame carried a zero
payload, so adding the integrity check immediately failed it. The
second is the more interesting one — **a control case has to be
genuine in every respect the parser checks, or it stops being a
control.**

## Downsides accepted

- One IPC round trip per 64 bytes (above).
- `FRAME_MAX` of 512 in the stack: smaller than an Ethernet MTU, so a
  full-size frame is still refused — counted honestly as oversize
  rather than silently truncated.
- The staged frame is single-slot: one reader, one frame at a time.
  Correct for a stack that processes frames one at a time, and a real
  constraint the moment anything wants concurrency.

## Future implications

- **UDP is now unblocked** and inherits a working frame path.
- The zero-copy receive is the obvious next performance milestone,
  and it is a ring-ownership design, not an optimisation to sprinkle
  in.
- `FRAME_MAX` and the driver's chunk size are the two numbers to
  revisit when something needs an MTU-sized frame.

## Appendix: ports are NOT a kernel capability

Phase 7's plan recorded UDP's real question as "who may bind a port
is a capability question, not a protocol one". Having looked at what
that would mean here, the first half needs qualifying before UDP is
built on it.

A port must **not** become a kernel capability kind. Minting a
`CapObj::UdpPort` would require the kernel to know what UDP is,
which is exactly the knowledge ADR-0030 spent a milestone keeping out
of it — and the kernel has no way to enforce anything about a
namespace it cannot see. The port namespace belongs to `netstackd`,
and the unforgeable thing a client holds is its **endpoint** to
netstackd, which is already a capability in this system's terms.

There is also a concrete obstacle that decides the v1 shape: **IPC
v1 gives a server no caller identity.** `ipc::recv` returns the
request words, a landed capability and the inline message — nothing
that says who called. So netstackd *cannot* distinguish two clients
arriving on one endpoint, and no amount of capability modelling
changes that. Per-client port ownership therefore requires
per-client endpoints, which requires netstackd to serve several
endpoints at once — a real piece of work with its own design
(one endpoint per client, or a caller-identity word in the ABI).

### Correction (C's review of v0.15.0)

The paragraph above overstated its case, and the correction is worth
more than the original claim.

**Per-binding AUTHORITY does not require per-client endpoints.** A
shared endpoint can hand back an opaque, unforgeable **token** at
BIND; later operations present it, and possession of the token *is*
the authority. No pid, no caller identity, and no UDP knowledge in
the kernel — which is exactly this system's model of a capability,
implemented at the service layer instead of the kernel one.
Deliberately passing the token to another program is then delegation,
which is a feature rather than a hole.

What I actually demonstrated was narrower: **per-client IDENTITY** —
non-transferable, "this process and no other owns this port" — is not
available on a shared endpoint under IPC v1, because `recv` reveals
nothing about the caller. That remains true and is a real constraint
on any policy phrased in terms of *who* rather than *what is held*.

The distinction matters because ArenaOS is built on the second kind
of answer everywhere else: a capability is authority by possession,
not proof of identity. Reaching for identity here was the wrong
instinct, and the token is the design that fits the rest of the
system.

So UDP v1 issues an rngd-backed random handle at BIND and treats
possession as the authority, and enforces bind-once as a NAMESPACE
rule, which is a separate question from authority. What it does not
do is claim non-transferable ownership — stated as a limitation, and
now as a deliberate one rather than an imagined impossibility.
