# ADR-0114 — Graphical service identification: protocol-level identification over the granted BadgedEndpoint (B-GFX-1)

**Status: Proposed** (decision record; not accepted, not implemented)
**Date:** 2026-10-27
**Milestone context:** V1 — Native Graphical Launch Foundation; unblocks the native C graphical proof and later SDL3 work.
**Related decisions:** ADR-0074 (badged endpoints), ADR-0083 (native startup ABI v2), ADR-0090 (badge-authenticated Desktop sessions), ADR-0111 (ASTR v2), ADR-0112 (native C runtime profile), C2.1-B-GFX-1-HANDOFF.md.

## 1. Problem

Startup ABI v2 (ARST) grants a graphical application a set of capability descriptors in its one-page startup record. All graphical descriptors carry role **Other (5)**. Today the record contains, among others, a per-session BadgedEndpoint and a session SharedRegion (the client surface), but kind and rights alone cannot say which BadgedEndpoint is *this application's Desktop session* and which SharedRegion is *its owned surface*. Two descriptors of the same kind and rights are indistinguishable by contract.

The current code locates them by slot constants (`SERVICE_ENDPOINT=1`, `SURFACE=2`, `CLOCK=3` in `userspace/desktop/src/app_client.rs`; `V2_ENDPOINT_SLOT=1`/`V2_SURFACE_SLOT=2` in `userspace/desktop/src/bin/desktop.rs`). That is exactly what B-GFX-1 forbids: never identify graphical capabilities by slot, descriptor order, or a guessed index.

Kind/rights counting (`arena_startup_count_kind`) remains an existence check only; it does not establish purpose.

## 2. Options considered

| # | Option | ABI/wire change | Identifies at launch? | Author | Verdict |
|---|---|---|---|---|---|
| 1 | **Protocol-level identification over the granted BadgedEndpoint** (this ADR) | none (ADSK-v1 preserved; one new Identify opcode/frame, versioned) | after one bounded round trip | Arena 1 Desktop + clients | **Chosen for V1** |
| 2 | Signed-manifest launch layout (APB1 declares expected descriptor layout; launcher checks) | none in ARST; new manifest field | yes, if launcher checks | packaging + launcher | Rejected for V1: positional trust if any launcher skips the check; the task does not authorize signed-package/manifest changes; does not replace option 1 anyway |
| 3 | Future ARST extension (per-descriptor service tag or new role) | yes (ARST version, ADR, Rust/Python/C coordination) | yes | architecture + all runtimes | Deferred: needs an ADR and M9 (`tools/startup_abi.py` CAP_MAX=4) fixed first; largest change |

Option 1 is the C2.1-B-GFX-1 near-term recommendation. It needs no kernel or ARST change, no packaging authorization, and no privilege path.

## 3. Decision

**Identify the Desktop endpoint and the application-owned surface by protocol-level identification over the BadgedEndpoint that the launcher already granted, verified against the capability authority the application legitimately holds.**

- Authority comes from possession, not from names or numbers: the trusted launcher (Desktop itself, per ADR-0090) mints one BadgedEndpoint for this session and grants the application exactly one writable session SharedRegion. Both are already in the ARST record; the runtime locates them by kind and rights over the *verified* record — never by slot.
- The application sends one versioned Identify request over that endpoint. The kernel delivers the call with the endpoint's badge in receive metadata (`SYS_IPC_RECV_BADGED`, word 3); Desktop matches the badge to exactly one live session (ADR-0090 `select_live_badge`: unknown, stale, duplicate, or dead-owner badges fail closed) and answers only for that session.
- A numeric reply manufactures no authority: the reply names a surface by kind/generation/rights, and the client accepts it only if it cross-checks against a SharedRegion it already holds (`SYS_CAP_DESCRIBE`: kind, object generation, exact rights). Mismatch, unknown generation, or rights mismatch → typed refusal, no state change.
- Missing, duplicate, malformed, stale, ambiguous, wrong-rights, wrong-version, and dead-service cases are all refused explicitly.

## 4. Mechanism

### 4.1 Client-side discovery (no slots)

After `arena_startup_gate` verifies the ARST record and the live cap table, the client runtime scans the verified descriptors (descriptor `i` corresponds to child slot `i+1` for `i` in `0..cap_count`, but slot numbers play no role in identification):

- **Desktop endpoint candidates:** descriptors with kind `BadgedEndpoint (12)`, role `Other (5)` as recorded, rights exactly `WRITE|COPY|DESTROY` (attenuated: cannot receive, bind, or mint).
- **Surface candidates:** descriptors with kind `SharedRegion (7)`, role `Other`, rights exactly `READ|WRITE|COPY`.

Refusals (fail closed, before any graphical call):
- zero endpoint candidates → `ERR_NO_ENDPOINT` (the app has no Desktop session).
- two or more endpoint candidates → `ERR_AMBIGUOUS_ENDPOINT`; nothing is guessed.
- zero surface candidates → `ERR_NO_SURFACE`.
- two or more surface candidates → `ERR_AMBIGUOUS_SURFACE`.
- rights that do not match exactly → `ERR_WRONG_RIGHTS`.

Exactly one endpoint and exactly one surface candidate proceed. The cap's slot is used only as the operand to `SYS_IPC_CALL`, never as its identity.

### 4.2 Identify frame (ADSK-v2, version 2)

ADSK-v1 (wire version byte `1`, opcodes 1..=12, 64-byte frames, `userspace/desktop/src/wire.rs`) is preserved verbatim. Identification uses **ADSK version 2**, a new opcode `Identify = 13`, same 64-byte frame size, canonical reserved-byte rules:

Client request frame (version 2):
- `b[0..4]` = `"ADSK"`, `b[4]` = 2, `b[5]` = 13 (Identify)
- `b[6..8]` = reserved zero
- `b[8..16]` = client protocol version (`u16` LE at 8..10, rest zero) — the version the client speaks
- `b[16..64]` = reserved zero

Service reply frame (version 2), written back into the same inline buffer:
- `b[0..4]` = `"ADSK"`, `b[4]` = 2, `b[5]` = 13
- `b[6]` = status: `0` = OK; nonzero typed refusal (section 4.5)
- `b[7]` = reserved zero
- `b[8..16]` = echoed session identity: the endpoint badge's generation-scoped session serial (`u64` LE) — lets the client detect a reply spliced from another session
- `b[16..24]` = named surface descriptor: `kind` (`u8` at 16, must be 7), `rights` (`u8` at 17, must include READ|WRITE), `generation` (`u64` LE at 18..26) of the session's surface
- `b[26..64]` = reserved zero

Optionally, when the reply slot carries a capability (`out[2] != CAP_NONE`), Desktop may transfer an attenuated copy of the session surface cap; the client installs it only if `SYS_CAP_DESCRIBE` reports kind 7, rights READ|WRITE|COPY, and generation matching `b[18..26]`, then destroys any extra copy it cannot account for. An unexpected landed cap on a refused path is destroyed per existing disposal rules.

### 4.3 Desktop service handling

On `SYS_IPC_CALL` to the BadgedEndpoint:
1. Read the kernel-delivered badge (receive metadata word 3). `badge == 0` (plain endpoint) → legacy path, unchanged.
2. `select_live_badge(badge)`: exactly one live broker session whose held Process is live → proceed; unknown/stale/duplicate/dead → typed refusal, no session or surface state change.
3. Decode the Identify frame: magic, version 2, opcode 13, reserved bytes zero, client version supported → else `ERR_INVALID_FRAME` or `ERR_PROTOCOL_VERSION`, reply without mutation.
4. Look up that session's owned surface (one writable SharedRegion per session, per the ADR-0090/ADR-0075 reservation). If the session has zero or more than one surface → `ERR_AMBIGUOUS_SESSION` (Desktop-side invariant violation; refused).
5. Compose the reply: status OK, echoed session serial, surface kind/rights/generation; optional attenuated surface-cap transfer.
6. Destroy the landed request cap on every branch (malformed requests included — no landed authority survives a refusal).

### 4.4 Client-side acceptance

On a reply:
- status != 0 → typed refusal, no graphical state created.
- magic/version/opcode mismatch, reserved nonzero → `ERR_INVALID_FRAME`, refuse.
- echoed session serial does not match the serial the client associates with its granted endpoint → `ERR_SESSION_MISMATCH`, refuse (the reply is for another session).
- named surface kind != 7 or rights lack READ|WRITE → `ERR_WRONG_SURFACE_KIND`/`ERR_WRONG_RIGHTS`, refuse.
- the client's verified cap table must contain a SharedRegion with kind 7, rights READ|WRITE|COPY, and object generation equal to the reply's generation, and exactly one such match → confirmed as this session's owned surface. Zero or more than one match → `ERR_STALE_SESSION` / `ERR_AMBIGUOUS_SURFACE`, refuse.

Only after all checks does the client treat the surface as its owned presentation buffer and the endpoint as its Desktop session.

### 4.5 Typed refusal codes (client-visible)

| Code | Meaning | When |
|---|---|---|
| `ERR_NO_ENDPOINT` | no qualifying BadgedEndpoint in the grant | launch refusal |
| `ERR_AMBIGUOUS_ENDPOINT` | two or more qualifying endpoints | launch refusal |
| `ERR_NO_SURFACE` | no writable session SharedRegion | launch refusal |
| `ERR_AMBIGUOUS_SURFACE` | two or more writable surfaces | launch refusal |
| `ERR_WRONG_RIGHTS` | endpoint/surface rights not exact | launch refusal |
| `ERR_INVALID_FRAME` | bad magic/version/opcode/reserved | Identify round trip |
| `ERR_PROTOCOL_VERSION` | client or Desktop version unsupported | Identify round trip |
| `ERR_SESSION_MISMATCH` | reply echo names another session | Identify round trip |
| `ERR_STALE_SESSION` | badge/session no longer live or generation changed | Identify round trip |
| `ERR_WRONG_SURFACE_KIND` | reply names a non-SharedRegion surface | Identify round trip |
| `ERR_WRONG_RIGHTS` (reply) | reply surface rights not READ|WRITE | Identify round trip |
| `ERR_SERVICE_GONE` | Desktop process dead | kernel `STATUS_SERVICE_GONE` |

All refusals are immediate typed errors; none manufactures an endpoint, surface, or session.

### 4.6 Versioning and size validation

- Frame size is fixed at 64 bytes (ADSK). Shorter/longer is a framing error before decoding.
- Wire version 1 = ADSK-v1 (unchanged, no Identify); wire version 2 = Identify available. A client speaking v2 that receives a v1 reply, or a Desktop receiving a non-v2 Identify request, refuses with `ERR_PROTOCOL_VERSION` — no silent downgrade, no guessing.
- Reserved bytes (`b[6..8]`, `b[16..64]` unused regions) must be zero on both directions; nonzero reserved bytes refuse the frame.
- The client version field (`u16` at `b[8..10]`) is `1` for this design; Desktop answers only versions it implements.

### 4.7 Bounded waits

The Identify exchange is one synchronous `SYS_IPC_CALL`. The client retries only `STATUS_BUSY` with the existing bounded backoff budget (ADR-0071 style: `1000us << attempt` capped), then fails. There is no polling loop, no indefinite block, and no fallback path. If the client wants an absolute wall bound it may arm its private notification/timer around the call using its existing stream authority; the call itself remains synchronous and fail-fast.

### 4.8 Teardown

- Client closes its surface window (`Frame::DestroyWindow` over the endpoint), closes its stream endpoints, and destroys its client-side copy of the BadgedEndpoint (the cap carries DESTROY; destruction empties only the client's own slot).
- On client death, Desktop retires the session's surfaces and focus through held Process-cap liveness (existing ADR-0090/ADR-0060 mechanics) before slot reuse; stale surface generations cannot revive authority. A surface cap outliving its session is a stale-surface refusal on reuse.

## 5. Trust-boundary analysis

"Asking an arbitrary endpoint to identify itself does not prove it is Desktop." This design does not do that:

1. The endpoint the client calls is not arbitrary: it is the BadgedEndpoint the launcher (Desktop) itself minted and granted to this session. Badges are kernel-minted on `SYS_ENDPOINT_MINT`; a client cannot forge a badge or a badged cap (ADR-0074: badge 0 reserved, READ never carried by a badged cap, generation advanced on mint/destroy, stale caps refused).
2. The kernel authenticates the caller to Desktop: on `SYS_IPC_CALL` through a badged cap, the badge travels in receive metadata, not in caller-controlled words (words 0..1 are explicitly non-authoritative, per ADR-0090).
3. Desktop answers only for the badge's own session (`select_live_badge`: unique and live). It never names another session's surface.
4. The reply's surface is accepted only when it matches a capability the client already holds, verified by `SYS_CAP_DESCRIBE` (kind, generation, rights). A bare numeric surface ID from the reply is a hint, never authority.

The bootstrap authority therefore flows: launcher grant → client possession of Desktop-minted cap → kernel-authenticated call → Desktop's session-bound reply → client cross-check against its own held caps. No descriptor is trusted by slot, order, or index.

## 6. Impact table

| Area | Effect | Notes |
|---|---|---|
| Kernel | none | No syscall, wire, or cap-kind change. |
| ARST v2 / startup record | none | No version bump, no new role/tag. |
| Desktop service (`userspace/desktop`) | add Identify opcode handler (ADSK v2, opcode 13); badge→session lookup already exists (`session_auth.select_live_badge`); surface naming per session | All ADSK-v1 opcodes preserved; legacy badge-0 path unchanged. |
| Rust app clients (`app_client.rs`) | replace slot-constant location (`SERVICE_ENDPOINT=1` etc.) with kind/rights discovery over `arena_startup()->caps`, then Identify round-trip; slots used only as IPC operands | Existing audit pattern retained as a cross-check, not as identity. |
| Desktop bin (`desktop.rs`) | `V2_ENDPOINT_SLOT`/`V2_SURFACE_SLOT` constants become implementation locations only; session/surface binding by badge/generation | No change to the granted child profile (ADR-0090). |
| C runtime (`experiments/c-runtime/`) | add C binding(s) for discovery + Identify exchange over existing syscall wrappers; host test + guest app | No new syscalls, no mini-libc, no private allocator. |
| Launchers / packaging (APB1) | none | Grant profile already exists; no manifest change required for V1. |
| Tests | new guest suites: valid launch; descriptor reordering (discovery is order-independent); missing/ambiguous endpoint/surface; wrong rights; malformed frames; unsupported version; stale generation; focus/input isolation; teardown + repeated launch | Negative controls use the restored-mutation pattern where production cannot emit the state. |

## 7. Risks and downsides accepted

- One extra IPC round trip before the surface is usable; Desktop holds per-session surface state (already true under ADR-0090 sessions).
- The client must trust that the launcher granted exactly one endpoint and one surface; a misconfigured launcher fails fast with typed refusals rather than guessing.
- Identify adds a new code path to the Desktop service; it is covered by typed refusals and existing ADSK-v1 preservation, with regression suites.
- Does not solve identification before the first message (option 3's benefit); recorded as a known limitation revisitable via an ARST service-tag ADR.

## 8. Future implications

- Enables V1's signed native C graphical proof: the C app identifies its endpoint and surface through this contract, renders over the real Desktop path, receives real input, and tears down cleanly.
- Genuine SDL3 execution (separate compatibility milestone) would consume this: SDL's video backend maps the identified surface and publishes damage through the Desktop path; no slot-based constants (`SERVICE_ENDPOINT 1`, `SURFACE_SLOT 2`, `CLOCK_SLOT 3`, `DIAGNOSTICS_SLOT 4`) survive into that integration.
- If a future need arises to identify graphical capabilities *before* the first message, option 3 (ARST service tag) remains the documented path, gated on an ADR and on fixing `tools/startup_abi.py` M9 (`CAP_MAX = 4`).

## 9. Acceptance criteria (for M/C review)

- [ ] ADR-0114 reviewed by C and decided by M before any contract implementation.
- [ ] Desktop identifies exactly one surface per session; ambiguous/missing/stale cases refuse with the typed codes above.
- [ ] A native C client locates endpoint and surface by kind/rights (order-independent), completes Identify, and refuses on every negative case.
- [ ] No slot constants are used to identify graphical capabilities in client or Desktop code after implementation.
- [ ] All ADSK-v1 behavior and legacy badge-0 dispatch remain byte-identical on the wire.
