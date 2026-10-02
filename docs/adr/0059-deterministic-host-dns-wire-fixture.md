# ADR-0059: Controlled host DNS peer on the actual M7 virtual wire

Status: accepted for Phase-9 integration (2026-10-02).

## Context

An earlier historical test run reached 72/73 but timed out awaiting its second
external DNS query. A replacement reached 31/74 and received 29-byte negative
answers for four M7 boots. Neither failure is dismissed as a flake, and the
external resolver was not packet-trace root-caused. Guest DNS must still make
real virtio-net/SLIRP/UDP requests and reject a wrong transaction ID.

## Decision

The M7 raw IPC, independently linked native UDP client, and stack DNS API
query the exact bounded `example.com` A/IN question at `10.0.2.2:1053`,
which SLIRP routes to a separately bound `127.0.0.1:1053` host UDP socket.
The host actor validates the whole 29-byte request and answers with a
61-byte two-record A response. The second client uses distinct source port
5355; the stack's internal source is 5354. The unrelated post-driver-restart
ARP probe still resolves `10.0.2.3`. The host receipt requires three queries
on every M7-successful normal boot; extra or missing requests fail. Test and
stability harnesses prebind the socket before launching QEMU. Their existing
TCP peer remains separately bound; a single Bash coprocess owns both stable
fixture sockets. There is no in-guest DNS shortcut.

## Boundaries and negative control

This proves guest/host virtual network exchange, typed IPC and native-client
continuation across a 56-byte inline boundary, independent stack DNS parsing,
and a strict response-TXID check. It **does not prove public/external DNS**,
reliability of the external resolver, or arbitrary-name resolution. The host
actor is a test peer, not production infrastructure. `test_m9_host_dns_red.py`
sends a wrong TXID from that socket: the genuine guest fails M7 and halts;
the unchanged EFI boots GREEN when the actor is restored. QEMU exit zero alone
never counts as PASS. An extracted deployment must provide the peer or make an
honest headless/no-network boot; its boot instructions may not advertise
external DNS proof.

A historical pre-kernel firmware stall was observed; explicit ESP boot-drive
selection hardens the harness but does not establish a root cause for that
stall. Earlier incomplete/full-suite failures remain failed evidence until a
fresh complete run succeeds; no intermediate checkpoint qualifies Phase 9.
