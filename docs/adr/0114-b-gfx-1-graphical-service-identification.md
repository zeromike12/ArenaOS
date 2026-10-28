# ADR-0114 — Graphical service identification: discovery over the granted capabilities (B-GFX-1)

**Status: Proposed** (decision record; not accepted, not implemented)
**Date:** 2026-10-28 (corrected after C source review; see the correction log below)
**Milestone context:** V1 — Native Graphical Launch Foundation. The V1 client
(`experiments/c-runtime/app/c_gfx_app.c`) implements the discovery portion of
this ADR; the Identify round-trip is the not-implemented protocol extension and
is deliberately absent from V1 until M accepts this ADR.
**Related decisions:** ADR-0074 (badged endpoints), ADR-0083 (native startup ABI v2),
ADR-0090 (badge-authenticated Desktop sessions), ADR-0111 (ASTR v2), ADR-0112
(native C runtime profile), C2.1-B-GFX-1-HANDOFF.md.

## 1. Problem

Startup ABI v2 (ARST) grants a graphical application a set of capability
descriptors in its one-page verified startup record. All graphical descriptors
carry role **Other (5)**. The record contains, among others, a per-session
BadgedEndpoint and one session SharedRegion (the client surface), but kind and
rights alone cannot say *which* BadgedEndpoint is this application's Desktop
session and *which* SharedRegion is its owned surface: two descriptors of the
same kind and rights are indistinguishable by contract.

The current code locates them by slot constants
(`SERVICE_ENDPOINT=1`, `SURFACE=2`, `CLOCK=3` in
`userspace/desktop/src/app_client.rs`; `V2_ENDPOINT_SLOT=1`/`V2_SURFACE_SLOT=2`
in `userspace/desktop/src/bin/desktop.rs`). That is exactly what B-GFX-1
forbids: never identify graphical capabilities by slot, descriptor order, or a
guessed index.

Kind/rights counting (`arena_startup_count_kind`) remains an existence check
only; it does not establish purpose.

## 2. Options considered

| # | Option | ABI/wire change | Identifies at launch? | Author | Verdict |
|---|---|---|---|---|---|
| 1 | **Discovery by kind and rights over the verified ARST record** (V1 mechanism, this ADR) | none | yes — existence + uniqueness check | Arena 1 Desktop + clients | **Chosen for V1** |
| 2 | Protocol-level identification over the granted BadgedEndpoint (ADSK version 2, opcode `Identify = 13`, versioned) | none (ADSK-v1 preserved; new Identify frame) | after one bounded round trip | Arena 1 Desktop + clients | **Described here, not implemented in V1** |
| 3 | Signed-manifest launch layout (APB1 declares expected descriptor layout; launcher checks) | none in ARST; new manifest field | yes, if launcher checks | packaging + launcher | Rejected for V1: positional trust if any launcher skips the check; the task does not authorize signed-package/manifest changes; does not replace option 1 anyway |
| 4 | Future ARST extension (per-descriptor service tag or new role) | yes (ARST version, ADR, Rust/Python/C coordination) | yes | architecture + all runtimes | Deferred: needs an ADR and M9 (`tools/startup_abi.py` CAP_MAX=4) fixed first; largest change |

Option 1 is the C2.1-B-GFX-1 near-term recommendation. It needs no kernel or
ARST change, no packaging authorization, and no privilege path.

## 3. Decision

**Identify the Desktop endpoint and the application-owned surface by discovery
over the capability authority the application legitimately holds: the verified
ARST v2 record.**

- Authority comes from possession, not from names or numbers: the trusted
  launcher (Desktop itself, per ADR-0090) mints one BadgedEndpoint for this
  session and grants the application exactly one writable session
  SharedRegion. Both are already in the ARST record; the runtime locates them
  by kind and rights over the *verified* record — never by slot.
- Exactly one endpoint candidate (kind `BadgedEndpoint (12)`, role `Other (5)`,
  rights exactly `WRITE|COPY|DESTROY`, attenuated: cannot receive, bind, or
  mint) and exactly one surface candidate (kind `SharedRegion (7)`, role
  `Other`, rights *including* `READ|WRITE|COPY`) are required. Zero candidates
  → typed refusal; two or more → `ERR_AMBIGUOUS_*`; nothing is guessed.
- The cap's slot is used only as the operand to `SYS_IPC_CALL`, never as its
  identity.

The optional protocol extension (option 2) — one versioned Identify request
over the endpoint, with the kernel-delivered badge authenticating the session
on the serve side and a surface cross-check via `SYS_CAP_DESCRIBE`
(`kind/object_id/rights`, rights including `READ|WRITE`, not exact) — is
described in section 4.2 for the record and is **not implemented in V1** while
this ADR remains Proposed.

## 4. Mechanism

### 4.1 Client-side discovery (V1, implemented)

After `arena_startup_gate` verifies the ARST record and the live cap table,
the client scans the verified descriptors (descriptor `i` corresponds to child
slot `i+1` for `i` in `0..cap_count`, but slot numbers play no role in
identification):

- **Desktop endpoint candidates:** descriptors with kind `BadgedEndpoint (12)`,
  role `Other (5)` as recorded, rights exactly `WRITE|COPY|DESTROY`.
- **Surface candidates:** descriptors with kind `SharedRegion (7)`, role
  `Other`, rights including `READ|WRITE|COPY`.

Refusals (fail closed, before any graphical call):

- zero endpoint candidates → `ERR_NO_ENDPOINT` (the app has no Desktop session).
- two or more endpoint candidates → `ERR_AMBIGUOUS_ENDPOINT`; nothing is guessed.
- zero surface candidates → `ERR_NO_SURFACE`.
- two or more surface candidates → `ERR_AMBIGUOUS_SURFACE`.
- rights that do not include `READ|WRITE|COPY` → not a surface candidate;
  if any surface candidate exists but none qualifies → `ERR_WRONG_RIGHTS`.

Exactly one endpoint and exactly one qualifying surface proceed. The client
then `SYS_SHARED_MAP(surface_slot, 1)` (writable flag 1; any other value is
rejected by the kernel), checks the page count with `SYS_SHARED_PAGES` (V1
geometry needs 301 pages: one offset page at 4096 plus 300 u32-pixel pages for
640×480), and renders u32 pixels at offset 4096. No `SYS_ENDPOINT_BADGE`
client query is ever made: the badge is the server's business
(ADR-0074 amendment, Phase 11.5).

### 4.2 Identify round-trip (protocol extension, NOT implemented in V1)

ADSK-v1 (wire version byte `1`, opcodes 1..=12, 64-byte frames,
`userspace/desktop/src/wire.rs`) is preserved verbatim. Identification uses
**ADSK version 2**, a new opcode `Identify = 13`, same 64-byte frame size,
canonical reserved-byte rules:

Client request frame (version 2):

- `b[0..4]` = `"ADSK"`, `b[4]` = 2, `b[5]` = 13 (Identify)
- `b[6..8]` = reserved zero
- `b[8..16]` = client protocol version (`u16` LE at 8..10, rest zero)
- `b[16..64]` = reserved zero

Service reply frame (version 2), written back into the same inline buffer:

- `b[0..4]` = `"ADSK"`, `b[4]` = 2, `b[5]` = 13
- `b[6]` = status: `0` = OK; nonzero typed refusal
- `b[7]` = reserved zero
- `b[8..16]` = echoed session identity: the endpoint badge's
  generation-scoped session serial (`u64` LE)
- `b[16]` = named surface kind (must be 7), `b[17]` = rights (must include
  `READ|WRITE`), `b[18..26]` = surface object id (`u64` LE)
- `b[26..64]` = reserved zero

No optional capability transfer in V1: the reply carries `CAP_NONE`; a landed
cap on any path is destroyed per existing disposal rules.

Desktop service handling (serve side): the kernel delivers the call with the
endpoint's badge in receive metadata (`SYS_IPC_RECV_BADGED`, word 3);
`select_live_badge` matches the badge to exactly one live session (unknown,
stale, duplicate, or dead-owner badges fail closed) and answers only for that
session. The named surface in the reply is cross-checked by the client against
a `SharedRegion` it already holds (`SYS_CAP_DESCRIBE`: kind 7, object id
equal to `b[18..26]`, rights including `READ|WRITE`); mismatch → typed
refusal, no state change.

### 4.3 Failure model

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

## 5. Correction log (2026-10-28, M's correction pass after C source review)

1. Audit actual API semantics from source, not syscall numbers:
   `SYS_SHARED_MAP(slot, writable_flag 0|1)` — passing 225 (a page count) as
   the writable flag is rejected; `SYS_CAP_DESCRIBE(slot)` returns
   `[kind, object_id, rights]` (object ids are descriptive, never handles);
   `SYS_SHARED_INFO(slot)` returns `[object_id, pages]`; `SYS_ENDPOINT_BADGE`
   is serve-side only (the badge of a landed badged cap, for the endpoint's
   own server).
2. Surface geometry: u32 pixels at offset 4096; 301 pages for 640×480;
   no RGB888 from offset zero.
3. Removed the `SYS_ENDPOINT_BADGE(ep, ep)` client query; trust via kernel
   badge + object-ID cross-check on the serve side.
4. Fail closed: result checks everywhere, no zeroed-output-as-success,
   validated event frames, disposed unexpected transferred caps, original
   failure preserved when cleanup fails.
5. `SYS_IPC_CALL` is not inherently bounded — no client-side deadline;
   external supervision for bounds.
6. Wire spec: `CAP_DESCRIBE` returns kind/object_id/rights, not generation;
   surface-field bounds fixed; no optional cap transfer in V1; rights must
   include `READ|WRITE|COPY`, not exact; protocol extension preserves ADSK-v1;
   obsolete "Python startup max 4" removed (Phase 16 fixed it); launcher slot
   placement is not client identity.
7. Test design: host controls for syscall failure, malformed replies, wrong
   identities, unexpected caps, layout bounds, and cleanup failures are
   runnable; guest cases stay blocked until this ADR is accepted and a
   bootable graphics-capable mainline ESP can be built in this sandbox.

## 6. Known limitation

The Identify round-trip (section 4.2) is specified but not implemented, and
ADR-0114 is Proposed, not accepted. V1 graphical clients therefore identify by
discovery (section 4.1) only. If M accepts this ADR, the Identify opcode and
the client/server frame handling can be added without changing the kernel or
the ARST.
