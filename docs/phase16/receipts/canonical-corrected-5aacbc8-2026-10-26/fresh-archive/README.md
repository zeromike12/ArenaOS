# Fresh corrected demonstration-archive verification (post-sealing sidecar)

Archive: `ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz`
Bytes: 5,016,910
SHA-256: `106088522042fb501a00554442ae97133127e53a9141d5e95069f112c9a9898b`

Repository artifact (after the packaging commit is pushed):
<https://github.com/zeromike12/ArenaOS/blob/arena/2b4ae1a5-arenaos/docs/phase16/artifacts/ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz>

External sidecar in the repository:
`docs/phase16/artifacts/ArenaOS-Phase16-Demonstration-5aacbc8.tar.gz.sha256`

This verification sidecar is intentionally outside the sealed
archive: recording the archive hash inside it would change the
archive. Machine-readable record: `run.json`.

## Fresh run

The sealed archive was checked against its sidecar, listed, and
extracted fresh under `/home/user/p16f`. Generated QMP/vcon socket
paths were 87–91 bytes, below the 108-byte limit.

- `verify_package.py` static exit 0: 225 package files verified plus
  `SHA256SUMS` (226 files total), all digests and manifest identities
  matched, the 100-file source subset matched its manifest, and the
  ESP inner EFI was byte-identical to the packaged EFI.
- `verify_package.py --guest` exit 0 in 15 s with the pinned patched
  QEMU wrapper: the extracted `source/tools/test_m15_guest.py` booted
  the extracted EFI/ESP on both `max` and `qemu64` CPU profiles and
  printed `Phase-15 native guest qualification: PASS`.
- Extracted artifacts after the guest run remained:
  - EFI `7dcee4426edc4b2a204592a5680c27c5ee91d51e3231705641df2ca0d7e2bfe6`
  - ESP `04ffaf4681e92fe87886bdcff29b175df032ae9d88bd6cae59b34ad070ea674a`
- The archive SHA-256 was unchanged before and after verification,
  and the exact sealed bytes were copied into the repository.

Files here: sidecar check, static log, successful guest log, both
extracted-boot raw serials and QEMU stderr logs, `package-listing.txt`,
and copies of the package `MANIFEST.json` / `SHA256SUMS` for review
without extracting.
