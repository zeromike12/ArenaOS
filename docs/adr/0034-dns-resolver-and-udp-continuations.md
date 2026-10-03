# ADR-0034 — DNS A lookup at the service boundary, UDP continuations, and revocation

*Milestone 7.5. Status: accepted.*

## Problem

M7.4 sent a DNS query over UDP but did not **resolve** a name: the client
checked only its transaction id and the response bit, and the service
kept only the first 56 response bytes. A typical 61-byte answer could
not even be inspected in full. M7.4 also left a UDP checksum gap.
While tracing the handle lifecycle for chunk access, another problem
became apparent: CLOSE freed a binding slot but BIND reused the slot's
original random handle. The old holder would regain authority when a
new binding occupied that slot. That contradicts ADR-0033's possession
model: revocation cannot mean "wait until somebody rebinds".

## Decision

- `netstackd` owns an explicit `DNS_OP_LOOKUP` for a bounded dotted
  ASCII name. It sends a DNS A/IN question to the slirp fixture's
  resolver, gets a **fresh transaction id from rngd per lookup**, and
  matches source address and port, transaction id, response flags,
  question and answer owner before returning an IPv4 address. A
  malformed name is refused before any packet or entropy request.
  `netd` still does not know DNS, IPv4 or UDP; the kernel does not
  know ports or names.
- A DNS request occupies one temporary INTERNAL UDP binding while it
  waits (port 5354, reserved from public BIND). The normal receive
  demultiplexer delivers its answer. The answer is decoded with a
  bounded, allocation-free parser; compression pointers must refer
  backwards, jumps and lengths are bounded, and CNAME chasing is
  deliberately not attempted. The wire proof asks an actual external
  resolver about `example.com`, not an in-tree responder.
- UDP inboxes now keep complete payloads up to the stack's **512-byte
  frame** bound. `UDP_OP_RECV` delivers the first 56 bytes and full
  length; `UDP_OP_RECV_CHUNK` takes the **same bearer handle** and an
  offset and returns up to 64 continuation bytes. It cannot be used
  without the handle. Eight inboxes live in BSS, not on the one-page
  user stack. Each is still single-slot; an undelivered datagram is
  replaced by a newer arrival. A delivered long datagram is pinned
  until the last chunk, CLOSE, or the next RECV; arrivals while pinned
  are dropped rather than overwriting bytes under the reader.
- UDP checksums are computed on send, including IPv4 pseudo-header,
  and nonzero checksums are verified on receive. A zero inbound field
  remains legal for IPv4 (no checksum), but the sender never emits
  zero-as-absent: a computed zero is transmitted as `0xffff`.
- Reusing a previously issued binding slot draws a **new** token from
  rngd. If entropy fails the bind fails, not falls back to the old
  handle. A zero or duplicate startup draw fails closed too. No PID
  check, kernel UDP capability, or non-transferable identity is added.

## Proof

`arptest` drains the *real* 61-byte UDP response across two IPC
messages with the same bearer; a forged handle and invalid offset are
refused; CLOSE+rebind yields new authority and the old handle stays
revoked. It then requests `DNS_OP_LOOKUP`, verifies a nonzero A result
from the real resolver, and checks malformed names are refused. A
synthetic guest self-test verifies outgoing odd-length UDP checksum,
accepts a valid nonzero incoming checksum, refuses and counts a
single-byte payload corruption, and accepts the IPv4 zero-checksum
exception. Host tests exercise compressed answers and negative parser
cases (wrong transaction, truncation, NXDOMAIN, compression cycle,
wrong question). Existing M1–M7 boots remain required.

## Limits and implications

- This is an A/IN resolver, not a recursive DNS implementation: one
  question, a direct A owner, no CNAME chase, IPv6, EDNS, DNSSEC,
  cache, TCP fallback, or search domains. A truncated, oversized,
  malformed or unsupported answer is a typed refusal, not an invented
  address. The reply bound is 470 UDP payload bytes inside a 512-byte
  Ethernet frame; full-MTU reception still needs its own milestone.
- DNS sends once and has a two-second **deadline**, not a polling loop.
  No blind retry after an ambiguous send or driver restart. The slirp
  resolver, guest IP, and reserved source port are fixture facts, not
  network configuration. A future configurable network requires DHCP
  and resolver configuration explicitly; no fake generality here.
- UDP bearer tokens are random but not kernel capabilities: the service
  enforces them. Deliberate transfer remains delegation. A source tuple
  plus a fresh 16-bit DNS id protects against accidental/mismatched
  replies; it is not a claim of authenticated DNS or resistance to an
  on-path adversary. That belongs to a different threat-model decision.
