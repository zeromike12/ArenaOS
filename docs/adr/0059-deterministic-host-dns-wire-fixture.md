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

## Targeted evidence to date (not a full-suite or Phase-9 qualification)

`tools/test_m7.py` passed with three separately logged 29-byte host queries
and 61-byte replies (`build/udp-dns-test-m7.log`). A real M7 boot with
`ARENA_DNS_WRONG_TXID=1` halted on the wrong response ID; the same EFI and
restored actor then booted M7 GREEN with three receipts under
`tools/test_m9_host_dns_red.py`. One first attempt of that test failed its
GREEN stage at the shell because the test itself provided an empty feeder;
the guest was not shut down. The feeder was corrected to the established
marker-paced default, and the complete RED/GREEN test passed. The full
historical/graphics suite is still pending at the time of this entry.

The first complete replacement run passed **73/73 current test suites** on
this controlled peer; it is not the final Phase-9 suite because the live
compositor/client/input and their future focused tests do not yet exist. A
first provisional two-boot stability attempt passed boot 1 but failed boot 2
when Bash killed the *coprocess wrapper* and left its `network_fixture.py`
Python child bound to 1053. Its owned orphan was identified and terminated;
`coproc ... { exec python3 ...; }` now replaces that wrapper so `$TCP_PEER_PID`
is the actual socket owner. A fresh two-boot run passed **2/2**, with three
per-boot host DNS receipts, the existing QMP display pixels and no leftover
listener. This is not an exact-final-EFI 100/100 qualification.

The subsequent complete Phase-9 suite passed 79/79, its exact EFI passed
100/100 graphical boots with exactly three host DNS receipts each, and the
independently extracted archive included and exercised the controlled peer.
This closes the integration gate without claiming public DNS access.
