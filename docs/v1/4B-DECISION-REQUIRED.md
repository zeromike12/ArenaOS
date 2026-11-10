# Requirement 4b — bounded decision note

**Disposition: OPEN; decision required from M/C.** No waiver is implied by this note or by the current same-process probe.

## Unproven property

The current authenticated Desktop session rejects a **live SharedRegion capability belonging to a different concurrently live Desktop session** before any graphical state is created or changed, while the current session remains bound to its original kernel badge. A stale capability from a retired generation is also untested. These are not interchangeable properties.

## Existing coverage (distinct claims)

- The ordinary production Identify exchange sends `CAP_NONE`, receives `CAP_NONE`, validates the reply against the held session surface, and is recorded separately.
- `foreign-identity` changes the surface object ID by one and runs the production identity validator. This is a **synthetic identity mutation**, not a capability.
- `alternate-streamset-identity` obtains the actual same-process StandardStreamSet SharedRegion descriptor with `CAP_DESCRIBE` and passes its distinct object ID through the production identity validator before any map/window operation. It is a **live same-process alternate SharedRegion**, not another session's graphics surface.
- `unexpected-transfer` remains a separate negative control: it attempts a non-CAP_NONE Identify request with the client's own surface and checks refusal/disposal plus retention of the client's surface. It does not cover a foreign surface and does not alter the ordinary CAP_NONE exchange.
- Predicate-only layout controls and the real 301/300 layout grant are separate evidence; neither establishes 4b.

## Missing fixture operation / why existing authority is insufficient

The refusal client receives one badge-bound endpoint and its own surface, plus its standard-stream capabilities. It receives no second session's surface capability. The Desktop has no existing IPC operation that gives one session another session's surface; its retained session record is not a held foreign capability. The client also has no memory-pool authority with which to create a substitute SharedRegion.

A genuine guest-level case therefore needs a second session's live surface capability to be presented to the production dispatch under the first session's badge, using an operation separate from Identify. No current grant, stream, or syscall can do that without introducing additional capability authority or a new test-only delivery path. Identify must remain `CAP_NONE` request / `CAP_NONE` reply; no successful Identify transfer is proposed.

## Smallest implementation if explicitly authorized

The smallest strong guest fixture is a test-only refusal launch that also holds one concurrently live peer session's **exact READ|WRITE|COPY surface capability**, then sends it in a separate existing ADSK `Create` request over the original badge-bound endpoint. The production Desktop badge gate rejects a landed SharedRegion whose identity/rights do not match that authenticated session before normal service dispatch or window creation. Today the Desktop drops the creator cap after spawning the peer and has no operation that gives this other session's cap to the refusal client. A fixture therefore needs a test-only retained/duplicated peer-cap source and an added refusal-client startup grant. It would prove the peer capability's live `CAP_DESCRIBE` identity, keep the ordinary Identify exchange `CAP_NONE`/`CAP_NONE`, assert refusal before map/render/window publication, and verify both sessions' cleanup. Full rights are needed to exercise the foreign-session identity check rather than merely the wrong-rights branch. This changes capability retention and the refusal client's startup grant (additional authority); it is **not authorized or implemented** by this work. A read-only identity-only probe would be weaker and would not exercise that production dispatch. No new syscall or successful Identify transfer is proposed.

## Narrow waiver alternative for M

If M decides the additional test-only grant is not justified, M may record a waiver scoped **only to requirement 4b**. The gate's exact `scope` value is: `requirement 4b only: no test-only cross-session/stale SharedRegion capability is granted to the refusal client; ordinary Identify remains CAP_NONE/CAP_NONE`. The ordinary Identify exchange remains `CAP_NONE`/`CAP_NONE`; the listed synthetic-ID, same-process live-SharedRegion, and attempted-own-capability-transfer controls remain in force. The waiver must be explicit in `M-WAIVER.json`, attributed to M, tied to the exact source commit and EFI SHA-256, timestamped, and include a rationale. This document is not that waiver. Until a qualifying fixture or such an M-recorded decision exists, 4b remains OPEN and blocks overall PASS.
