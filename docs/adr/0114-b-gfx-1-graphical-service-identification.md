# ADR-0114 — Graphical service identification: discovery, then Identify (B-GFX-1)

**Status: Accepted (architecture)** — M approved the architectural direction
(Discover → Identify → validate already-held surface → proceed).
**Implementation: pending** — not yet implemented in V1; no merge, no release,
no V2/V3 or SDL3 integration authorized by this acceptance.
**Date:** 2026-10-28 (architecture accepted 2026-10-29; implementation
backlog follows)
**Milestone context:** V1 — Native Graphical Launch Foundation.
**Related decisions:** ADR-0074 (badged endpoints), ADR-0083 (native startup ABI v2),
ADR-0090 (badge-authenticated Desktop sessions), ADR-0111 (ASTR v2), ADR-0112
(native C runtime profile), C2.1-B-GFX-1-HANDOFF.md.

## 1. Accepted architectural decision

**Discover → Identify → Validate → Proceed.** The V1 graphical client:

1. **Discovers** candidate capabilities by kind + rights over the verified
   ARST v2 record — never by fixed slot, descriptor order, or guessed index.
2. **Performs the Identify round-trip** over the discovered BadgedEndpoint
   (ADSK version 2, opcode 13) before treating the candidate surface as its
   graphical presentation resource.
3. **Validates** the Identify reply against a capability it already holds:
   surface object ID, kind, and required rights must match a `SharedRegion`
   the client already holds (`SYS_CAP_DESCRIBE`: kind 7, object ID equal to
   the reply's object ID, rights including `READ|WRITE`).
4. **Only then** proceeds with the graphical demonstration (Create, Damage,
   Poll, …).

Discovery-only identification is **diagnostic scaffolding**. It locates
candidates but does not resolve B-GFX-1, does not implement an accepted
identification contract, and does not satisfy V1's positive acceptance case.
Identify is mandatory before graphical use; it is not optional and not outside
V1. It remains unimplemented until the Desktop server and V1 client are
updated under this accepted contract.

## 2. Approved principles

- **Trusted launch profile:** The Desktop itself (per ADR-0090) mints one
  BadgedEndpoint for this session and grants exactly one writable session
  SharedRegion. The launcher establishes which endpoint candidate the
  application may contact: exactly one endpoint candidate with rights
  `WRITE|COPY|DESTROY` must exist in the verified ARST record. Missing or
  ambiguous candidates are refused — the client never probes arbitrary
  endpoints.
- **Discovery never proves service purpose:** Kind/rights uniqueness over the
  verified record establishes existence and uniqueness of candidates only.
  It does not prove that a candidate is the Desktop session for this
  application, nor does it generalize to arbitrary startup environments.
- **Identify is mandatory:** The client must not render into or publish the
  candidate surface until Identify succeeds and the reply validates against a
  held capability. On Identify failure, the client fails closed; it does not
  fall back to discovery-only operation.
- **Desktop authenticates via kernel badge:** The kernel delivers the badge
  in receive metadata (`SYS_IPC_RECV_BADGED`, word 3). `select_live_badge`
  matches it to exactly one live session whose process is live; unknown,
  stale, duplicate, or dead-owner badges fail closed.
- **Desktop names only that session's surface:** The reply names the session's
  SharedRegion object ID, kind, and rights.
- **Client validates against held authority:** The full returned surface
  object ID, kind, and specified rights are matched against an already-held
  capability. A numeric object ID is descriptive and does not create
  authority.
- **No client hidden-badge query:** The client never calls `SYS_ENDPOINT_BADGE`
  (serve-side only, ADR-0074 amendment).
- **No server READ grant, no new syscall, no weakened kernel check.**
- **No capability transfer for Identify:** `CAP_NONE` on both request and
  reply; unexpected transferred capabilities are rejected and disposed through
  the existing safe disposal paths.
- **ADSK-v1 compatibility:** ADSK-v1 (wire version byte `1`, opcodes 1..=12,
  64-byte frames) is preserved verbatim. Identification uses ADSK version 2.

## 3. Exact supported launcher profile

The trusted launcher (Desktop) grants, per session:

| Capability | Kind | Role | Rights | Count |
|---|---|---|---|---|
| BadgedEndpoint | 12 | Other (5) | exactly `WRITE|COPY|DESTROY` (14) | exactly 1 |
| SharedRegion (surface) | 7 | Other (5) | includes `READ|WRITE|COPY` (7) | exactly 1 |

Both are role `Other` descriptors in the verified ARST v2 record. The
runtime locates them by kind and rights — never by slot. The endpoint slot is
used only as the operand to `SYS_IPC_CALL`, never as its identity.

## 4. Identify wire protocol (ADSK version 2, opcode 13)

ADSK-v1 is preserved verbatim. Identification uses **ADSK version 2**,
same 64-byte frame size, canonical reserved-byte rules.

**Client request frame (version 2):**

- `b[0..4]` = `"ADSK"`, `b[4]` = 2, `b[5]` = 13 (Identify)
- `b[6..8]` = reserved zero
- `b[8..10]` = client protocol version (`u16` LE; supported value **1**)
- `b[10..64]` = reserved zero

**Service reply frame (version 2), written back into the same inline buffer:**

- `b[0..4]` = `"ADSK"`, `b[4]` = 2, `b[5]` = 13
- `b[6..8]` = reserved zero
- `b[8..16]` = echoed session identity: the session's generation-scoped
  serial (`u64` LE). Descriptive only — not a secret the client must already
  know, and not independent authentication of Desktop.
- `b[16]` = named surface kind (must be 7 = SharedRegion)
- `b[17]` = named surface rights bitmask (must include `READ|WRITE`; COPY is
  required to land the surface cap but the reply field reports the session's
  surface rights)
- `b[18..26]` = surface object ID (`u64` LE): the object ID of the session's
  SharedRegion. Descriptive, not authority.
- `b[26..64]` = reserved zero

**Status encoding:** Typed service errors are carried in the existing IPC
reply status word (`SYS_IPC_CALL` out[0]), not in a frame byte. Bytes 6 and 7
are reserved zero in both request and reply. A successful reply requires both
kernel success (`SYS_IPC_CALL` returns 0) and service success (out[0] == 0).
The client decodes the payload only after both succeed.

**Status policy:** `0` = OK; `STATUS_BAD_ARG` for a malformed request frame
(bad magic, bad version, nonzero reserved, unsupported opcode);
`STATUS_SERVICE_GONE` if the Desktop process is destroyed while the call is
queued or in hand ("unknown", not "did not happen"); `STATUS_BUSY` only as a
kernel queue refusal (bounded retries, section 6).

**Rights policy:** The client matches the full returned surface object ID,
kind, and specified rights against an already-held capability
(`SYS_CAP_DESCRIBE`: kind 7, object ID equal to `b[18..26]`, rights including
`READ|WRITE`). Mismatch → typed refusal, no state change.

**Reserved-byte policy:** `b[6..8]` and `b[26..64]` must be zero in both
directions; any nonzero reserved byte makes the frame malformed and the call
is refused with `STATUS_BAD_ARG`.

**Capability-transfer policy:** `CAP_NONE` on both request and reply. A
landed capability on any path is destroyed per existing disposal rules. The
client never receives a new cap from Identify.

## 5. Failure model

- `SYS_IPC_CALL` is blocking and has **no client-side deadline**; a client
  distinguishes queue refusal (`STATUS_BUSY`), a normal typed reply, server
  death (`STATUS_SERVICE_GONE`), and a live-but-unresponsive service.
  Bounds come from external supervision, never from a client-side deadline.
- Only kernel `STATUS_BUSY` retries are bounded (`IPC_BUSY_RETRIES = 512`
  attempts with `1000us << min(attempt, 5)` backoff); a service status is
  never retried — that request was already processed.
- Fail closed: `create_window`/`damage`/`teardown` check results; no
  zeroed-output-as-success; event frames validated (magic, version, opcode,
  handle, reserved bytes); unexpected transferred caps disposed; the original
  failure is preserved when cleanup fails.

## 6. Implementation status

| Component | Status |
|---|---|
| ADR-0114 architecture | Accepted (M, 2026-10-29) |
| Desktop Identify handler | Not implemented |
| Canonical client bindings | Not implemented |
| c_gfx_app Identify exchange | Not implemented |
| V1 guest proof | Not executed |

This acceptance does **not** implement the contract, qualify V1, or authorize
a merge or release. Implementation follows under separate review.

## 7. Correction log

1. 2026-10-28: M's correction pass after C source review — restored the
   agreed V1 contract sequence (Discover → Identify → Validate → Proceed);
   discovery-only is diagnostic scaffolding, not the final contract.
2. 2026-10-28: Status-encoding clarification — bytes 6 and 7 reserved zero;
   typed service errors use the existing IPC reply status word, not a frame
   byte; CAP_NONE on both request and reply; no optional capability transfer.
3. 2026-10-29: M accepts the architectural direction; ADR-0114 status changes
   from Proposed to Accepted (architecture). Implementation remains pending.