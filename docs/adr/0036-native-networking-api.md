# ADR-0036 — Native userspace networking library with transferable bearers

*Milestone 7.7. Status: accepted after full regression and artifact-bound boot qualification.*

## Problem and decision

M7.6's `userspace/net.rs` wrapped only TCP. A caller still had to know
raw IPC words, byte order, datagram continuation offsets and status
codes to use ARP, ICMP, UDP or DNS. Merely publishing those numeric
constants is not an API library. Expand the same no_std module into
`Client` (a caller-held stack endpoint), `UdpSocket` and `TcpConnection`
(bearer-bearing values), `Address`, `Datagram`, `TcpState` and typed
`Error`. The module maps ARP resolution, ping, DNS A, UDP bind/send/
whole-datagram receive/close, and TCP active-open/poll/write/read/close/
release. Production uses synchronous IPC; a transport trait enables
strict host contract tests of the actual module without faking a guest
network.

**Authority is possession, not identity.** The library never mints a
bearer or installs a UDP-specific kernel capability. `bind_udp` and
`tcp_open` return the service's rngd-backed handles. `handle()` may
intentionally be shared or passed to another client; `adopt_udp` and
`adopt_tcp` wrap a received value without claiming it is valid. Only
the service can authorize it on each operation. Explicit `close` and
`release` revoke the server authority and clear the wrapper locally;
there is no hidden Drop-time IPC, which might fail without a caller to
observe it. A wrapper is not a security boundary and transferring a
bearer deliberately delegates authority (ADR-0033).

The library checks caller input sizes before IPC, decodes byte order,
validates reply lengths/states, and automatically drains UDP
continuations under the *same* bearer into a caller-owned, fixed-size
470-byte buffer. An undersized or contradictory reply is a protocol
error, not an invented success. A DNS name is bounded to dotted ASCII
A lookup. A caller must still drive TCP via POLL and handle a
stop-and-wait write/close; the library does not promise nonblocking IPC
for ARP, ICMP, DNS or UDP receive, nor passive/listening TCP.

## Alternatives and boundaries

A POSIX `socket` facade would suggest listening, arbitrary-sized
streams, platform-wide file descriptors, and an identity/ownership
policy this service does not have. An opaque, non-transferable handle
would falsely treat the client wrapper as authority instead of the
service-issued bearer. A Drop-based revocation would silently mask
network errors. Reject all three. This is the native *bounded* API for
the Phase 7 v1 service, not an unrestricted production TCP stack or a
promise of DHCP, IPv6, tap interoperability or general Internet reach.

## Evidence

Host contract tests compile the real no_std module and ABI and inject
an exact-call fake transport: wrong op, argument or request bytes fail;
ARP/DNS/ping status translation, UDP source/offset/reassembly and
revocation, TCP state/error/revocation and malformed replies are covered.
The existing guest network exercise uses the public library for ARP,
ICMP, DNS and TCP and makes an additional UDP query through it against
the real slirp DNS peer. It checks the live source tuple, transaction
ID, data beyond the inline boundary and a revoked handle; the host TCP
fixture still verifies the stream and FIN independently. A no-peer
boot still SKIPs TCP explicitly. Qualification: `tools/run_tests.sh`
18/18 suites, `tools/stability_loop.sh 100` 100/100 boots of the final
image, receipt in `build/stability-receipt.txt`.
