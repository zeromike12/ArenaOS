# Fresh demonstration-archive verification (post-sealing sidecar)

Archive: `ArenaOS-Phase16-Demonstration-d314d20.tar.gz`
Bytes: 4,855,009
SHA-256: `08786f7900581b08e65acef7466c98d56e05587e6f873c46d2c1ed0cd22225fc`

This sidecar is intentionally outside the sealed archive: recording
the archive hash inside it would change the archive. Machine-readable
record: `run.json`.

## Attempts

1. **Attempt 1 — FAIL before guest boot (harness path limit).** The
   first fresh extraction used a long sandbox prefix. Static package
   checks passed and artifacts matched, but QEMU refused to start
   because the generated vcon Unix socket path was at least 108 bytes.
   No guest assertion ran. The complete failed log is retained as
   `fresh-extract-attempt1-path-too-long.log`. The package verifier and
   package README were then amended to fail closed with a short-prefix
   instruction and the archive was resealed. (The failed attempt used
   the pre-amendment archive; only the verifier/README changed, not
   the EFI/ESP or evidence.)
2. **Attempt 2 — PASS.** The sealed archive above was extracted under
   `/home/user/p16f` (socket path 91 bytes). `verify_package.py`
   static exit 0 (202 package files, all digests, manifest identities,
   ESP inner EFI byte-identical). `verify_package.py --guest` exit 0:
   the extracted `source/tools/test_m15_guest.py` booted the extracted
   EFI/ESP on both CPU profiles and printed
   `Phase-15 native guest qualification: PASS`.

Extracted artifacts after the guest run remained:

- EFI `850cce0da36daa5d51452a4c4c4f01ae446b09272de293b3e14ad258f3ccaa79`
- ESP `65b489016527a49e394ab6e7a57cb592045a3c80b55cceed7610aa448cb519d9`

Files here: successful `fresh-extract.log`, both extracted-boot raw
serials and QEMU stderr, `package-listing.txt`, and copies of the
package `MANIFEST.json` / `SHA256SUMS` for review without extracting.
