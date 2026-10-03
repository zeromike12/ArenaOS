# Phase 8.5 design-only review ledger (2026-09-30)

**Status:** host-side model/measurement, **not** guest installation, an implemented syscall,
production reference accounting, durable fsd commits or Phase 8.5 qualification.
The last deployable/qualified artifact remains Phase 8.4. C approved the v1
payload-fit direction, 17→18 Notification increase, one coarse 8.5 marker, and
the two-slot/fresh-ID/revoke/page-budget/fail-stop design directions. ADR-0054
and ADR-0055 are **Accepted architecture** after C's final review; the
measurements here are still **not** implementation or qualification proofs.

Reproduce in this checkout (Python stdlib, checked-in AFS1 parser and bundled 8.4
scratch template; no Rust toolchain, new kernel image or private key):

```sh
python3 tools/test_phase85_ref_model.py
python3 tools/test_phase85_design_capacity.py
```

* Independent host transition model: **76** stable-boundary capspace/queued-cap
  conservation checks, separately checked pin owners, Waiting/Delivered/Replied/
  Failed/caller-first/server-first/full-receiver/revoke/last-ref/slot-reuse paths.
  Three deliberately omitted hooks — `send_enqueue`, `reply_enqueue`,
  `caller_sweep` — caused **RED** count mismatches at the predicted boundary;
  normal runs were GREEN. The model is *not* linked to future kernel registry
  hooks. ADR-0055 requires the independent **production-kernel** walker and
  deliberate mutation test again when that primitive exists.
* Actual archived AFS1 geometry: **8 MiB, 16,384 sectors, 11 initially used**.
  Offline synthetic maximal-size object table: historical 8.1/8.2 budget 19,
  8.4 budget 8, plus two installed and three activation records = **32/32**;
  `afs1.audit` clean, **99 allocated, 16,285 free sectors**. The fourth
  activation file would be object **33**, refused by a no-mutation preflight
  despite free sectors. This is a *capacity* fixture using synthetic bytes;
  it is not a signed/guest-verified namespace or a sequence of real fsd
  CREATE/WRITE commits. The combined mathematical maxima 19+8+2+4=33
  cannot all fit simultaneously without GC or a scope/bound change. ADR-0054
  records C's **accepted** conditional fourth decision (typed NO_SPACE
  before CREATE, prior platter byte-exact and usable); no bound was raised.
* Source-anchored cap schedule: existing manager 20 boot caps, accepted +2
  sources = 22, four conservatively reserved resident Process handles =26;
  old dynamic Image plus one held child Process = **28/32** during PREPARE;
  after teardown, commit landed Image plus attenuated local copy = 28/32;
  fresh LAUNCH/reply or one separately scheduled readiness worker yields an
  upper projection **29/32**. Packaged 5 inherited + owned buffer + landed
  marker + provisional Image = **8/32**. These depend on serialized cutover
  and **one unretired** dynamic child (now accepted system-wide; exit alone
  does not retire it).
  They are **not measured guest high-water**:
  the future 8.5 integration must instrument actual manager/verifier cap
  occupancy, process/record/frame peaks and the refusal boundary; a failure
  cannot be excused by this host model.

The accepted exact IPC words/messages, AINS latest-repeat/downgrade/conflict
rules, encoded PREPARE token/replay protocol and crash cuts are specified in
[ADR-0054](0054-installed-images-and-atomic-upgrade.md). The structural
registration TCB limit and production conservation proof requirements are in
[ADR-0055](0055-capability-gated-dynamic-image-registry.md). The earlier
actual 648-byte ELF/840-byte signed APKG v1 size proof is separately recorded
in [payload-fit evidence](0054-elf-v1-evidence.md). **Do not describe any of
these host design proofs as successful guest Image registration.**
