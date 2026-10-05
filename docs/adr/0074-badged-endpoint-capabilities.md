# ADR-0074 — Badged endpoint capabilities

Status: accepted; implemented on `arena/phase11-desktop-maturity` (Phase 11.2).

## Problem

Phase 11 needs userspace objects that are named by capabilities: file and
directory handles served by `filesd`, surfaces, chooser grants. With plain
endpoint caps a server can only tell *that* someone called, not *which of
its objects* the caller was given. Encoding an object id in the message
would make a forgeable number into authority.

## Decision

A new capability kind, `CapObj::BadgedEndpoint { eid: u16, generation: u16,
badge: u32 }` (the variant fits the existing 16-byte `CapObj`; measured
layout unchanged).

* **Minting.** `SYS_ENDPOINT_MINT(endpoint slot, badge, rights)` (call 48)
  requires a plain endpoint cap with READ — the serve side. Badge 0 is
  reserved for unbadged calls. The minted cap carries WRITE and optionally
  COPY, never READ, so it can never receive, reply, mint or bind, and
  `cap::serves_endpoint` never counts it. It lands in the minter's first
  free slot; the server hands it out through ordinary delegation (IPC
  reply/send cap, spawn grants).
* **Delegation.** Copy, move, IPC transfer and landing preserve the object,
  including the badge, and apply the existing rules unchanged: COPY on the
  source, rights only attenuate, amplification is refused.
* **Delivery.** `SYS_IPC_CALL` resolves either a plain endpoint cap
  (badge 0) or a badged cap. The badge travels in the call slot and the
  server reads it with `SYS_IPC_RECV_BADGED` (call 49: blocking or not;
  output `[w0, w1, landed-cap-slot, badge]`). Plain `RECV`/`TRY_RECV` are
  unchanged.
* **Object generation safety.** Endpoints carry a 16-bit generation
  advanced on every mint and destroy. A badged cap is honoured only while
  its generation matches the live endpoint; after the endpoint is destroyed
  and the index re-minted, every earlier badged cap is refused (and
  `SYS_CAP_DESCRIBE` refuses it). Plain endpoint caps keep their historical
  semantics.
* **Badges are not rights.** The kernel never interprets a badge. A server
  must treat the badge as a key into its own table (object index plus its
  own generation, and the per-object rights it granted when minting), and
  must refuse badges it did not issue or whose object is gone. Numeric
  equality with some other value never grants anything: only possession of
  a minted cap produces a badge on the server's side.

## Proofs

`m11:badged_endpoint` (boot suite): serve-side-only minting; badge 0,
READ and non-WRITE mint refused; transfer into another process and an
attenuated copy keep the badge; delegation without COPY and amplification
refused; a call through the badged cap delivers exactly its badge;
badged caps never serve; destroying and re-minting the endpoint index makes
earlier badged caps stale; both processes tear down frame-exactly.
`tools/test_m11_event_red.py` removes the generation check and the
serve-side mint check in turn and requires the guest to report the failure.

## Consequences

`filesd` (Phase 11.6) serves file and directory handles as badged caps on
one endpoint; per-handle rights live in its table, keyed by badge.

## Amendment (Phase 11.8): a minted badged cap may carry DESTROY

IPC transfer copies a capability, so a server that replies with a freshly
minted badged cap keeps its own copy. With mint rights limited to WRITE and
optional COPY, the kernel refused to destroy that copy (no DESTROY right,
not IPC-landed): `filesd` held one slot per OPEN for ever and its 64-slot
space filled after a few dozen opens; every later mint failed and the
explorer reported "Too many open items". Found by
`tools/test_m11_explorer.py` at its first Delete.

Mint rights are now WRITE with optional COPY and DESTROY. DESTROY only lets
a holder empty its own slot; it names no endpoint authority (a badged cap
still never carries READ and never serves). `filesd` mints WRITE | COPY |
DESTROY and destroys its copy after every reply. `m11:badged_endpoint` now
also proves that a cap minted without DESTROY cannot be destroyed by its
minter and that one minted with DESTROY can.
