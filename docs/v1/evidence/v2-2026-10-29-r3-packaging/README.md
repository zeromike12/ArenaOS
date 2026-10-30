# Packaging results for v2-2026-10-29-r3 (kept outside the sealed archive)

`package` and `verify` describe the archive, and the archive cannot contain a
field that describes itself: adding the receipt changes the bytes it names.
So the results for the packaging step live here, beside the sealed directory,
while the archive holds the evidence and receipts for every stage that was
executed against the candidate.

| File | Meaning |
|---|---|
| `package.json` | ledger/gate receipt for the seal (file count, digest, identities) |
| `verify.json` | gate receipt for the fresh-extraction verification |
| `aggregate-final.txt` | `tools/v1_gate_aggregate.py --mode full` output: V1 QUALIFICATION: PASS |
| `v2-2026-10-29-r3.tar.gz.receipt.txt` | copy of the external receipt published next to the archive |

The digest named here is for whichever archive the last `seal` produced; the
authoritative copy is the receipt beside the archive itself.
