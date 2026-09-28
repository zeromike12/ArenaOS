# ADR-0035 — TCP active-open v1: bounded, client-driven progress

*Milestone 7.6. Status: accepted; full regression and 100/100 artifact-bound boot qualification passed.*

## Problem

DNS (7.5) completes the datagram path, but Phase 7 still owes TCP and a
native userspace network API. A synchronous IPC server cannot park in a
connection's receive loop while serving other requests; a fake CONNECT
that blocks until the peer replies is not an asynchronous interface. Nor
can a 512-byte frame receiver honestly claim an unrestricted TCP stream.

## Decision

Build an active-open TCP client in `netstackd`, behind service-issued
rngd-backed bearer handles; the kernel and netd remain unaware of TCP.
A bounded client API in `userspace/net.rs` exposes OPEN, POLL, WRITE,
READ and CLOSE. OPEN transmits SYN and returns immediately once ARP has
resolved (ARP is separately bounded); POLL drives one receive/deadline
step through netd and returns state, never spinning. One connection is
supported in v1, no listening. TCP segments use the IPv4 pseudo-header
checksum, a random initial sequence and source port, and negotiate MSS
400, below the frame receiver's 512-byte limit. Writes fit one IPC
message. Inbound bytes occupy one bounded buffer and are read in chunks.
Only in-order data advances the receive sequence; out-of-order/duplicate
segments are acknowledged but not misdelivered. SYN, data and FIN
outstanding segments are retransmitted with the SAME sequence on a
bounded RTO; a connection that exhausts its retry limit fails typed.
Unknown-outcome `STATUS_SERVICE_GONE` does not trigger blind resend:
TCP's sequence-numbered retransmission handles uncertain delivery.

## Proof and scope

A real Linux host socket is the peer through slirp's 10.0.2.2 gateway:
the guest opens asynchronously, writes a pinned request, receives and
verifies a 200-byte position-dependent response across IPC messages,
and closes with FIN. The host actor checks the request and EOF; kernel
witnesses NIC interrupts and frame-exact teardown. Host parser tests
supply malformed header, wrong tuple/checksum and a valid control.
The actor runs in the normal QEMU fixture paths, including persistence,
crash boots and the 100/100 loop; these gates reject a missing peer or
a guest SKIP. A separate ordinary no-peer boot requires an explicit
SKIP and clean shutdown rather than a false TCP PASS.

## Limits and deferred work

No passive listen, multiple connections, reordering queue, congestion
control, window scaling, SACK, path MTU discovery, or arbitrary MTU
frames. A 400-byte advertised window and 400-byte MSS bound the peer;
excess/out-of-order data is not delivered. A poll-driven API is
asynchronous from the caller's perspective but the service has no
independent receive worker: clients must POLL to progress. This is a
bounded TCP client, not a general-purpose production TCP stack; a
future milestone must deliberately lift these limits before claiming
general-purpose TCP support. A host actor proves real interop, not security of
plaintext TCP or general Internet connectivity. `userspace/net.rs` is
a narrow typed client helper at M7.6; ADR-0036 expands it into the
native networking API in M7.7 before Phase 8.

## Interrupt and receive-ring correction uncovered by TCP

Three receive buffers are posted. With a single completed-frame hold,
ACK, data and FIN arriving in one burst lost the later entries; the
completed-buffer FIFO now preserves order, holding each buffer until
its last IPC chunk has been read. Separately, netd's old RX/TX badges
were `0x6e01` and `0x6e02`. Badges are OR-merged **bit masks**, not
identifiers: these values share bits. Under a burst, an RX badge could
satisfy a TX wait without a TX used entry, or consume the RX indication
before the waiting RECV checked its held frame. This caused intermittent
M7 hangs and TX completion failures in full-suite boots. RX and TX now
use disjoint single-bit badges, checked against each other and the
receive deadline at compile time; the wait examines the used ring and
returns when a completed frame was harvested. A green retry without
fixing this mask would have hidden the defect.

## Qualification

`tools/run_tests.sh` passed 17/17 suites (including parser and no-peer
checks). `tools/stability_loop.sh 100` passed 100/100 boots of the final
image, with a ready-bound real host peer per boot, guest handshake,
verified 200-byte stream, FIN/close/revocation and host request/EOF
proof. The artifact-bound receipt is `build/stability-receipt.txt`.
