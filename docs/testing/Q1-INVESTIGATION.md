# Q-1 investigation: Phase 16 registered attempt 1

## Summary

At qualified Phase 16 source
`5aacbc88c50d4565a7bb9f4075be27e40df27779`, the first complete registered
attempt failed:

- `tools/test_m84_crash.py`
- `tools/test_m9_client_death.py`

A second complete registered attempt passed 121/121.

This investigation reaches different evidence-based conclusions for the two
legs:

| Leg | Classification | Status |
|---|---|---|
| `m84-crash` | Preserved serial proves a host harness pacing race | **C recommends closure as resolved harness pacing; M's final decision pending** |
| `m9-client-death` | Mechanism plausible but not reproduced; original serial lost | **Remains open for A/C disposition** |

Nothing here declares Q-1 wholly closed. The m84 recommendation does
not close the separate `m9-client-death` leg, and the later
`m9-resources` observation documented below is a separate tracked issue
rather than a third Q-1 conclusion.

Primary Phase 16 evidence:

- `docs/phase16/receipts/canonical-corrected-5aacbc8-2026-10-26/registered/run.json`
- `docs/phase16/receipts/canonical-corrected-5aacbc8-2026-10-26/attempts/registered-attempt1-5aacbc8/README.md`
- `docs/phase16/receipts/canonical-corrected-5aacbc8-2026-10-26/attempts/registered-attempt1-5aacbc8/full-suite.log`
- `docs/phase16/receipts/canonical-corrected-5aacbc8-2026-10-26/attempts/registered-attempt1-5aacbc8/serial-m84-crash-recover-policy-close.log`

## `m84-crash`: query and shutdown raced service settlement

### Failing condition

The failed check was the `policy/close` same-platter recovery case. For the
disk state after that kill point, the suite requires both:

- `packaged READY`
- `pkg: INELIGIBLE`

The first proves the packaged service completed its boot settlement; the
second is the expected eligibility result for that recovered state.

### Preserved byte ordering

In the preserved failing serial (102,063 bytes):

| Serial event | Byte offset |
|---|---:|
| First shell prompt | 97,991 |
| Host-typed `pkg query app.test` | 98,371 |
| `packaged: boot ... namespace scan verified` | 100,742 |
| `pkg: INELIGIBLE` | 101,869 |
| Host-typed `shutdown` after second prompt | 101,885 |
| `packaged READY` | absent |
| `boot namespace scan refused` | absent |

The ordering is decisive:

1. The harness typed the package query after the shell prompt but before
   packaged-service settlement.
2. The service proceeded through its namespace scan and supplied the
   expected fail-closed/eligibility answer.
3. The second shell prompt appeared immediately after that answer.
4. The harness typed `shutdown`, and the guest exited before the separate
   `packaged READY` line was printed.
5. The unchanged assertion correctly failed because one required marker was
   absent.

The guest did not accept an ineligible package or expose a stale ALLOW. The
observed defect was that `recovery()` used shell prompts as a proxy for a
different service's readiness.

### Why only recovery had this hazard

The same file's general `boot()` helper already gated its first command on
both the prompt and `packaged READY`. `recovery()` was the outlier.

Two same-shape boots in `tools/test_m84_stage.py` also queried `pkg` after
only a shell prompt:

- corrupt-visible recovery;
- allocator-full reboot.

Both expect the refusal path, so waiting only for `packaged READY` would be
wrong. They need to wait for either success settlement or explicit refusal.

### Fix

`tools/mtest.py` marker tuples now support an any-of subgroup without
changing the historical flat-tuple conjunction semantics. The recovery boots
use:

```python
settle = (
    b"arena>",
    (b"packaged READY", b"boot namespace scan refused"),
)
```

This changes pacing only:

- no guest code changed;
- no eligibility assertion changed;
- no timeout was widened to mask a missing settlement;
- a boot that produces neither settlement marker still times out/fails;
- expected refusal boots can settle on the refusal line.

### Validation after the fix

In the rebuilt sandbox at runner commit `091d7cd` (with later testing-doc
commits not affecting these suites):

- `m84-stage`: PASS, 230.5 s
- `m84-crash`: PASS, 327.9 s
- Evidence run:
  `20261027T040136Z-suite-091d7cd-116348-c8fbfe`

All 16 `m84-crash` recovery boots in that run typed `pkg query app.test`
strictly after either `packaged READY` or
`boot namespace scan refused`; zero ordering violations.

The same suites also passed later inside full-selection batches
(`m84-crash` 296.3 s in chunk 4 and `m84-stage` 213.4 s in chunk 5).

Correction-pass validation on 2026-10-27 preserved this disposition: the
infrastructure controls explicitly pass flat-conjunction, nested any-of,
partial-settlement refusal, and `nth` alternative semantics, and both
`m84-stage` and `m84-crash` passed in each of the three uninterrupted
full attempts (including 208.5 s and 289.8 s respectively in the final
passing full). Assertions and guest deadlines remain unchanged.

### Review disposition

C recommends closing the `m84-crash` leg as a resolved harness pacing
race, with the settlement gate as the corrective action and the original
failure/assertions preserved above. M's final closure/acceptance
decision remains pending; this document does not declare the leg closed
unilaterally.

Whether packaged-service queries issued before settlement should queue
instead of using the current service behaviour is a product/API question for
A/C. The Q-1 evidence does not require a kernel or service change.

## `m9-client-death`: original evidence was overwritten

### What survives from attempt 1

The full-suite log records:

```text
[m9-client-death] GUEST FAILURE despite QEMU rc=0:
    fatal [arena ERROR halt]
...
assert rc == 0 and 'm7: RESULT PASS (2/2)' in serial
AssertionError
```

The raw failing serial does not survive. As the Phase 16 attempt README
states, the immediately following `test_m9_client_death_red.py` restored
GREEN invocation wrote the same `serial-m9-client-death.log` path and
replaced it.

That evidence loss is itself one root problem addressed by the new runner:
each run and attempt now has a unique evidence directory, and per-boot serial
copies cannot be replaced by a later suite or retry.

### Plausible mechanism from current code

`tools/test_m9_client_death.py` waits for all of the following before its
after-death screenshot:

- original child exiting without `DESTROY`;
- compositor retiring the dead child and presenting underlying pixels;
- root proving exact Process/region/map/comparator retirement;
- shell prompt.

In `kernel/kernel/src/entry.rs`, when the root observes the original child
has no live threads, it starts a 2,000,000-microsecond deadline. If the
copied SharedRegion does not reach the exact `(refs, pins) = (1, 0)` state
before that deadline, it halts with:

```text
graphics: dead original child never retired its copied region
```

This makes a timing/liveness hypothesis plausible, especially under TCG
because another graphics client continues polling. But without the failing
serial, the surviving evidence does not prove that this exact halt, rather
than another fatal path, occurred.

### Bounded reproduction in the rebuilt environment

Using the new evidence-preserving runner:

| Condition | Attempts | Result |
|---|---:|---|
| Idle host | 10 | 10/10 PASS |
| Two CPU hog processes | 5 | 5/5 PASS |
| Six CPU hog processes on a 2-CPU host | 5 | 5/5 PASS |

Run IDs:

- `20261027T041055Z-suite-091d7cd-117176-4de747`
- `20261027T041247Z-suite-091d7cd-117563-1135b0`
- `20261027T041356Z-suite-091d7cd-117789-b2c132`

In every preserved serial, retirement lines followed the exit line within a
few serial lines. The compositor/root order can swap, but neither was absent
and no `[arena ERROR halt]` appeared.

`m9-client-death` and `m9-client-death-red` also passed in full-selection
chunk 6. They passed again in all three corrected uninterrupted full
attempts on 2026-10-27. Those are additional passing observations only:
the original failing serial is still absent, so this leg remains open and
is not labelled fixed, harmless, or environmental.

### What evidence would discriminate the hypotheses next time

A future preserved failure should be classified from the raw serial, not
rerun away:

- **Deadline/liveness failure:** exit line present; one or both retirement
  lines absent; fatal halt says the copied region never retired.
- **Compositor presentation failure:** retirement lines present but QMP
  after-death pixels remain the dead client's surface.
- **Host harness failure:** required serial lines present, while QMP/fixture
  or screenshot handling fails outside the guest.
- **Different kernel failure:** `[arena ERROR halt]` carries a different,
  specific halt reason.

Boot JSON also records QEMU exit separately from semantic verdict, so a UEFI
fatal ResetSystem returning process 0 cannot be misread as success.

### Review disposition

Keep the `m9-client-death` leg **open**.

The evidence supports "not reproduced under the stated bounded conditions,"
not "harmless," "environmental," or "fixed." Do not widen the kernel's 2
second deadline blindly. If A/C want a product-side investigation, the next
preserved failing serial should drive it. Preserve that complete attempt
and classify it using the discriminators above before any retry is allowed
to become the story.

## Separate tracked issue: `m9-resources` transient process count

This observation arose during the 2026-10-27 correction pass. It is
**not** part of the original Q-1 pair and must not be used either to
close Q-1 or to infer the cause of `m9-client-death`.

In full attempt
`20261027T162612Z-full-30fe810-259917-b23647`, `m9-resources` failed
its exact assertion on this compositor diagnostic:

```text
[arena INFO  m9] compositor: live processes 17; ...
shared 3/96, pages 507/36864, maps 6/160
```

The test asserted the same line with 15 live processes. The preserved
serial shows boot processes still spawning/running around that
unsynchronized snapshot. The later settled measurements were exact:
shell records/processes went from 16/16 to 15/15 after the original
child retired, and shared regions/pages/maps and the retirement line
matched the expected 3/96→2/96, 507→488, and 6/160→4/160 transitions.

The sealed archive linked from `FINAL-REPORT.md` contains:

- the complete failed attempt at
  `runs/full-failed-attempt2-20261027T162612Z-full-30fe810-259917-b23647/attempts/m9-resources/`;
- ten passing preserved repeats at
  `runs/m9-resources-repeat10-20261027T182829Z-suite-30fe810-434082-eb79ed/`,
  each reporting 15;
- the next uninterrupted full, where `m9-resources` passed.

No assertion, kernel deadline, or product behavior was changed in
response. Those passes do **not** establish that the observation is
harmless, fixed, or proven environmental. Track it separately as open
reliability evidence. If it recurs, preserve the first attempt and
compare both the transient diagnostic ordering and settled retirement
accounting before deciding whether the product leaked resources or the
test needs a semantically synchronized snapshot—which would require
its own review rather than quietly broadening the expected value.

## Structural corrective actions now in place

- Unique `build/test-runs/<run-id>/` evidence per invocation.
- Unique attempt directories; `--repeat` records all attempts.
- Monotonic per-boot/per-build evidence allocation across harness processes.
- Raw serial, QEMU stderr, QEMU command/exit, semantic verdict, durations,
  artifact hashes, source status/diff, and backend identity preserved.
- A passing retry cannot convert a run with a failed attempt to green.
- `verify-run` fails closed if cited evidence is missing or altered.
- The corrected full and focused evidence are sealed in
  `docs/testing/evidence/T-correction-2026-10-27/` with an external
  SHA-256 sidecar and fresh-extraction verification log. C accepted
  the complete passing full plus complete failing attempts from the
  superseded runs as sufficient for this review; their passing attempt
  trees were not duplicated merely for closeout.
