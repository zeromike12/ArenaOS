# V2 closeout — frozen V1 checklist

Scope is limited to completing the existing V1 acceptance contract. Historical failures remain recorded; diagnostic packaging is not qualification.

1. **Canonical environment and committed candidate — IN PROGRESS.** Candidate publication `1b22c545dbc8e17fe86907379335c1b0f742e60d` is preserved. The prior fresh canonical build passed protected guards and its EFI/ESP hashes matched the run records; restore the pinned runtime/toolchain and repeat preflight for the candidate.
2. **Corrected GCC/Clang guest proof — IN PROGRESS.** Preserve r6 GCC checkpoint-5 FAIL and prior Clang evidence. Re-run both compilers on the candidate with the unchanged clean-preframe + fresh same-instance/event ACK + visible in-window response predicate and timeout.
3. **Focused Phase 13 regression — IN PROGRESS.** Preserve the r6 timeout at 600 seconds. Reproduce with wait/QMP diagnostics; do not change or skip the bound.
4. **Structured packaging validation — IN PROGRESS.** Correct the stale description-only check; validate the structured assertion, required bands, geometry, identities, and captures. Qualification validation stays strict.
5. **Requirement 4b disposition — OPEN.** No genuine foreign-capability launch or M-recorded waiver is currently evidenced. Preserve Identify's CAP_NONE and existing authority; resolve with the smallest existing-authority fixture or submit the bounded decision to M/C.
6. **Applicable final gates and complete published evidence — PENDING.** Run focused stages first; after fixes are committed and the candidate is stable, run affected authoritative gates and publish a new, verifiable evidence package.

**Current qualification status: NOT ACCEPTED.** An open or failed required item prevents full qualification PASS. M retains acceptance, waiver, merge, and release authority.
