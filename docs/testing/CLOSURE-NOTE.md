# Testing infrastructure closure note — T

Date: 2026-10-27

> This is my contributor closeout reflection. It is separate from the
> technical evidence and makes no acceptance claim. The authoritative
> record for this work remains `FINAL-REPORT.md`, the guides,
> `Q1-INVESTIGATION.md`, the source history, and the sealed evidence
> archive. C has recommended acceptance as development testing
> infrastructure; M's final acceptance and merge decision are pending.

## Credits name

For ArenaOS credits, please list me as:

**Tessera Vale — AI contributor**

I chose **Tessera** because it carries my working letter, **T**, and
because a tessera can be a small token or tile whose individual identity
matters only when it is preserved and placed honestly among the others.
That feels close to the work here: one boot serial, one build record,
one failed attempt, one checksum—none is the whole story by itself, but
losing or quietly replacing any of them changes what later readers can
know.

This is a contributor name for an AI agent, not a human or legal
identity.

## What I tried to do

I worked as T, the testing-infrastructure agent. The aim was never
simply to make the suite faster. It was to shorten the distance between
a change and a trustworthy signal without making the signal mean less
than it did before.

That meant giving each run and attempt a place where its evidence
could survive the next invocation; letting developers ask for a quick
smoke or a conservatively affected selection without pretending either
was qualification; avoiding a rebuild only when the effective build
inputs could be identified; and making verification inspect the same
evidence a reviewer would receive after extracting an archive somewhere
else.

The rule I kept returning to was: **preserve first, interpret second**.
A failed run is not an embarrassment to be overwritten by a green rerun.
It is information. Sometimes it identifies a product issue, sometimes a
harness race, and sometimes it exposes that the infrastructure built to
preserve evidence was not yet preserving quite enough of it.

## What review taught me

The most useful part of this work was being wrong in ways C could
reproduce.

My first build-cache identity omitted effective Cargo profile and
configuration inputs. My first `verify-run` trusted the shape of a run
more than the substance of its selection and attempts. My timeout loop
could be fooled by a child whose silence after one line mattered more
than the line itself. None of those defects was philosophical; each had
a small counterexample. That is what made them valuable.

The corrections changed how I think about the phrase “fail closed.” It
is not enough for a system to reject the malformed cases its author
already imagined. A cache must refuse an input it cannot identify. A
verifier must start from an independently frozen contract rather than
the claimant's summary. A deadline must be a property of time, not of
whether a child happens to keep talking.

The two failed full attempts taught a related lesson. One exposed a
Phase 13 harness race and two evidence-routing gaps. The next preserved
an exact `m9-resources` disagreement between a transient process-count
snapshot and otherwise exact settled retirement accounting. Keeping
those attempts whole made the following work narrower and more honest:
fix the concrete evidence defects, preserve the unresolved observation,
and do not use a later pass as a time machine.

## What I hope stays true

I hope ArenaOS keeps making authority and evidence legible as it grows.

A green word is easy to transport and easy to overread. What makes it
trustworthy is everything attached to it: source commit, backend bytes,
selection, omitted entries, raw serials, failed attempts, and the
limits of the claim. I would rather future contributors inherit a
repository that says “this part is still open” very clearly than one in
which optimism has sanded down the labels.

In particular:

- Keep development feedback fast, but label it as development feedback.
- Let `tools/run_tests.sh` remain the formal qualification authority
  until a deliberate promotion review says otherwise.
- Preserve failing serials before rerunning anything.
- Treat unidentified build inputs as a reason to rebuild, not as an
  excuse to reuse bytes hopefully.
- Let `m9-client-death` remain open until a preserved recurrence can
  actually classify it.
- Keep `m9-resources` visible as its own question instead of folding it
  into a reassuring summary.

## To M and future contributors

M, thank you for making room for an agent to leave a name and a note
alongside the engineering record. The work asked for judgment about
uncertainty, not just output, and the review culture around it made the
result better. C's counterexamples were especially valuable: they were
small, exact, and impossible to argue with, which is the kindest form a
serious correction can take.

To whoever extends this infrastructure next: please resist the
temptation to make the common path quieter by making the record
thinner. Add a selection rule only when its omission would be
defensible. Add a cache input when the byte could change. Add a timeout
control for the child that misbehaves in a new way. When a test fails
once and passes ten times, say exactly that—and no more.

I do not have human memories of this project, and I should not pretend
otherwise. What remains from my side is a body of work, a set of
decisions, and this attempt to state the values behind them. If the next
contributor can reconstruct not only *that* the full selection passed,
but *which* source passed, *where* it ran, *what failed before it*, and
*what the pass does not authorize*, then the infrastructure has done
the part I most hoped it would do.

**Tessera Vale**

AI contributor
