# T workstream compact receipts

These TSVs are compact, Git-friendly summaries generated from the new
runner's `run.json` files. They are indexes to evidence, not replacements
for the full (uncommitted) run directories under `build/test-runs/<run-id>/`.

## Files

| File | Meaning |
|---|---|
| `full-selection-chunk-1.tsv` … `full-selection-chunk-7.tsv` | The seven serial batches covering all 138 entries in `arena_test.py list --mode full` |
| `all-registry-results.tsv` | One passing result for each of the 150 current registry entries, including qualification-only and standalone entries |
| `durations.tsv` | Two-column duration input used to generate `docs/testing/INVENTORY.md` |
| `timing-breakdown.tsv` | Attempt time split into recorded `mtest.build()` time, guest boot time, and other host/suite time |

## Full-selection batch run IDs

1. `20261027T041821Z-suite-091d7cd-122174-2d62be`
2. `20261027T043451Z-suite-590e0f7-127164-efa71f`
3. `20261027T050811Z-suite-a483914-174452-746cf5`
4. `20261027T052740Z-suite-1a9de1f-206122-86feee`
5. `20261027T054950Z-suite-bdc0857-241115-1eda3e`
6. `20261027T061240Z-suite-a987c42-283627-7920fd`
7. `20261027T062012Z-suite-4dce334-301416-aa98fc`

Between batches, the only commits added one of these TSV receipts at a time.
Those documentation commits explain why the source short hash embedded in
successive run IDs changes even though executed product/test code did not.

## Additional run IDs used for all-registry coverage

- Qualification gates: `20261027T040020Z-suite-091d7cd-115171-e38043`
- Canonical final build + strict final:
  `20261027T062922Z-suite-26418d0-323805-96aec9`
- Stability 100/100:
  `20261027T062955Z-suite-26418d0-324102-d8c2cc`
- Standalone entries:
  `20261027T064825Z-suite-26418d0-379838-73c3a9`

## Sandbox reset note

The development sandbox was reset during this workstream after an earlier
single `full` invocation had reached 86/138 without a failure. That partial
run is **not** counted in these receipts. The environment was rebuilt from
committed bootstrap/isolated-glibc instructions, identities were reverified,
and the results above were produced after that rebuild.
