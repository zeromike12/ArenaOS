# Phase 16 → Arena 1 handoff (proposed Phase 17 only)

Date: 2026-10-26

Status: **HANDOFF FOR REVIEW. Phase 17 is not started and no SDL3 work
is authorized or claimed by this document.**

Arena 1 should treat this as the factual starting point if M/C later
accept Phase 16 and separately authorize genuine upstream SDL3 work.

## 1. Canonical runtime to build on

Use the one Phase 16 runtime at `experiments/c-runtime/` (promotion
out of `experiments/` remains an M decision). Do not fork a second
startup, allocator, stream, or thread layer for SDL.

Qualification-candidate source and evidence:

- Source: `d314d204aa86ec4018659f3f211cd3a488d30d87`
- Evidence: `docs/phase16/receipts/canonical-2026-10-26/`
- Final EFI: `850cce0da36daa5d51452a4c4c4f01ae446b09272de293b3e14ad258f3ccaa79`
- Final ESP: `65b489016527a49e394ab6e7a57cb592045a3c80b55cceed7610aa448cb519d9`

Canonical guest evidence covers GCC and Clang signed-APB1 execution
for C1/native, C2 multi-app, baseline x87/SSE2 FP, focused live stdin,
ASTR v2 (including v1 refusal), and Notification ambiguity refusal.
The registered historical suite is 121/121 and final-artifact
stability is 100/100 on the identified locally modified QEMU TCG
backend.

## 2. Runtime facts Arena 1 can rely on

Within the bounded ADR-0112 profile and `docs/compat` coverage:

- Validated ARST v2 startup reaches C `main` with stack/TLS/argv/env
  handling supplied by the canonical runtime.
- The shared allocator is the integration point; SDL must use the
  runtime allocator rather than a private Arena heap. Respect its
  measured limits and failure returns; do not raise limits just to get
  a green boot.
- Standard streams exist only when capability-granted. ASTR is v2
  only: three 1,024-byte rings, wrapping counters, v1 refused.
- Focused live stdin is real keyboard input through the owning
  AppInstance's granted stream, with Ctrl-D EOF. It is a stream, not a
  POSIX tty: no termios, signals, job control, or file-descriptor
  process model should be assumed.
- C11-subset threads are bindings over Arena kernel threads/sync.
  Process status is the last thread to exit; detached-thread lifetime
  must be managed explicitly. Thread count is kernel-bounded (four
  user threads per process in the current profile).
- Baseline x87/SSE2 ring-3 execution is available under ADR-0113. AVX
  is not supported; kernel code remains integer-only outside reviewed
  FP state leaves. Build SDL/runtime objects with the documented
  baseline profile and audit for AVX.
- Unsupported features fail closed. In particular, do not assume
  `fopen`/filesystem authority, floating-point `printf` conversions,
  wall-clock `time`, timed/recursive mutexes, TSS, dynamic loading,
  POSIX threads/signals/process APIs, sockets, audio, or GPU APIs.

Consult `docs/compat/C2-LIBC-COVERAGE.md` function-by-function. If SDL
needs a missing C function, add it to the canonical runtime with host
and guest tests or record an explicit refusal; do not add an SDL-only
mini-libc.

## 3. The blocker that must be decided first: B-GFX-1

Graphical service identification remains unresolved. ARST v2 graphical
descriptors can all carry role Other; kind/rights/counting can say
that a SharedRegion or BadgedEndpoint exists but cannot unambiguously
say “this is the surface” vs “this is the compositor.”

Hard rules carried forward:

- Never identify graphical capabilities by slot, descriptor order, or
  a guessed index.
- Count by kind/rights only as an existence check.
- Refuse ambiguity. The Notification policy is the local example:
  sole candidate only, otherwise `NONE` and an immediate typed error.
- Do not use a privileged serial/debug path as a substitute for a real
  graphical launch proof.

Options already compared in `docs/compat/C2.1-B-GFX-1-HANDOFF.md`:

1. **Protocol-level identification over the BadgedEndpoint**
   (recommended there for review): no ARST change; Desktop answers
   only when its per-session surface is unambiguous; the app uses only
   the authority named in the reply. Arena 1/Desktop must define and
   version the message and its refusal cases.
2. **Signed-manifest launch layout**: versioned APB1 layout checked by
   the launcher. Needs packaging authorization and still depends on
   every launcher performing the check.
3. **Future ARST extension/service tag**: self-describing but a wire
   change requiring an ADR and coordinated Rust/Python/C changes.

Phase 16 fixed the old M9 prerequisite mentioned in that handoff
(`tools/startup_abi.py` now matches the 7-descriptor native contract),
and Phase 16 now has a two-candidate Notification *guest* negative.
Those updates do not solve graphical surface/compositor identity.

M/C and Arena 1 need to choose and record the B-GFX-1 direction before
an SDL video backend can honestly claim graphical guest execution.

## 4. Suggested first Phase 17 gates (for later authorization)

If Phase 17 is authorized after Phase 16 acceptance:

1. **Decision gate:** record the B-GFX-1 choice (ADR if any wire or
   manifest format changes) and the exact SDL version/source hash.
   “Unmodified upstream SDL3” needs a precise definition of which
   upstream files may be configured vs patched.
2. **Build gate:** compile upstream SDL core against the canonical C
   runtime with baseline x86-64/SSE2, no AVX, static linking only, and
   an undefined-symbol report. Every missing symbol is either a
   canonical-runtime addition with tests or a documented refusal.
3. **Headless gate:** run a signed APB1 SDL test that initializes only
   subsystems that truthfully exist, reports version/features over a
   granted stream, and exits with a checked status. No fake video or
   audio success.
4. **Video gate:** after B-GFX-1, map the identified surface, publish
   damage through the identified compositor path, and verify owned
   pixels by QMP screendump plus lifecycle/resource teardown receipts.
5. **Event gate:** route real keyboard/pointer events only to the
   focused owning instance; prove no delivery to an unfocused second
   instance, mirroring the Phase 16 live-stdin isolation test.
6. **Audio gate:** only if an ArenaOS audio service/capability design
   is separately accepted. Until then SDL audio must report absent,
   not silently succeed.
7. **Regression gate:** rerun Phase 16 targeted suites, registered
   regressions, strict Phase 15, and final-artifact stability on any
   changed EFI/ESP. Keep new evidence separately labelled by commit
   and image hash.

## 5. Non-goals and warnings

- Do not turn the native C runtime into a Linux/POSIX personality.
- Do not add syscalls or kernel object types without the required
  architecture decision.
- Do not infer timers from wall clock, files from streams, a tty from
  stdin, or process semantics from `main` return while workers live.
- Do not claim timer-preemption FP preservation beyond the exact
  Phase 15/Phase 16 receipts.
- Do not start from the development EFI/ESP or development receipts;
  use the canonical source/images above if M/C accept them.

Phase 17 remains **proposed only** until M/C accept Phase 16 and give
separate direction.
