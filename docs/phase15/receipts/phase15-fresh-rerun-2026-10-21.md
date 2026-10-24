# Phase 15 fresh rerun evidence review

## Archive and integrity

The uploaded archive is preserved as
[`phase15-fresh-rerun-2026-10-21.zip`](phase15-fresh-rerun-2026-10-21.zip),
SHA-256
`edcf0253f9346c5cfa0f7f99ce819fe369955df40b4ef9d57a86cb80c955a11d`.
Its ZIP CRCs and paths were checked; the archive has 43 entries, no duplicate
or unsafe paths, and all 42 entries in its internal `SHA256SUMS` matched.
The embedded strict EFI, ESP, probe, OVMF code, and OVMF vars hashes were also
checked directly.

The archive includes `README-FOR-LUNA.txt`, which summarizes the run and
suggests inspection commands. It was treated as attached document content;
none of its commands or bundled scripts were executed during this review.

## Run identity

- The archive records current branch HEAD `391c333d0a9a4ab0e8686e24848c92747e2a56ef`
  and external kit SHA-256
  `134cbc228e39508ab93909b87c6f839c59d9e89d3030aae15ce672b651bf4993`.
- The kit's strict EFI and ESP match the pinned hashes
  `bbbc63df271c3d7d5109789369f72e093b02ef77c77ebe25273b6da4c00d238e` and
  `631cc6452ecbacf58b2418e54b9c509bb717cc2211580ebef63f9aa4c13d866f`.
  The CPL3 probe matches
  `1f057068b020baefe06f7f3b36bc0beeafcb879bd93d60c5ea66a3cce616c8d3`.
- OVMF code and vars match `7e34e3b83224968bb0714c0ebc88f75c5c37b4cc892c902e511ccc7a7c107f94`
  and `5d2ac383371b408398accee7ec27c8c09ea5b74a0de0ceea6513388b15be5d1e`.
  The included firmware source record identifies Debian sid `ovmf-generic`
  `2026.08+ds-2`.
- The QEMU record identifies Debian/Ubuntu QEMU 8.2.2
  (`1:8.2.2+ds-0ubuntu1.18`), binary SHA-256
  `8a35ccba41582fc6c38b9df85fc9e35fa1d42f414d2d7d8090ee9b2f5e7c0854`.
  The package list records the Ubuntu `qemu-system-x86` package SHA-256 as
  `14602e262627adac030329d2cecfee5f6c0938b34566ff2ea2d4c9a9eb02430d`. The
  QEMU binary and package files are not included, so those package and binary
  hashes cannot be recomputed from this archive. The included Ubuntu package
  policy is provenance evidence, not a copy of signed repository metadata.
- The kit manifest identifies strict-image build source commit
  `6586a0a18981932ae7048ca267fa425411f8e463`; the evidence records no ArenaOS
  or kit source changes. The included source-review files match their
  corresponding files at kit source commit
  `081bcd1a1cba1fc54fbf38a80f6dab2c95176c06`.

## Guest results

Both strict runs used explicit `-accel tcg` with the pinned EFI/ESP:

| CPU profile | QEMU exit | Semantic guest exit | M15 | Vector 19 |
| --- | ---: | ---: | --- | --- |
| `max,+nx,+smep,+smap` | 0 | 97 | FAIL (5/6) | Not delivered |
| `qemu64,+nx,+smep,+smap` | 0 | 97 | FAIL (5/6) | Not delivered |

Both serial logs report `CR0=0x80010033`, `CR4=0x668`, and the strict probe
falling through from MXCSR `0x1F00` to `0x1F01`, instead of fault status
`0x113`. The same-process sibling and independent process survived; the fault
slot was reclaimed and the resource baseline restored. The separate `#MF`,
`#UD`, MMX, isolation, and other reported guest cases passed. ArenaOS recorded
the suite failure and performed its declared clean shutdown. The semantic
runner returned 1; the QEMU exit code of 0 does not pass the guest assertion.

The serial hashes are `9b39ff8fe779a730c0227fe33c0243044d6ef04f45f167bf6765f8d8b2dd7ad4`
for `max` and `081b4a26b231535db8e1f93a1c02413fdc36a53356a767ab7ce257272c351f88`
for `qemu64`. The archive's `summary.json`, QEMU process records, serial logs,
and strict runner log agree on the failed vector-19 result.

This fresh result matches the official QEMU TCG source finding recorded in
[`qemu-tcg-xf-source-review.md`](qemu-tcg-xf-source-review.md): the SSE helper
sets MXCSR status flags but has no unmasked SIMD exception delivery path.
It does not reproduce an ArenaOS defect.

## KVM attempt and gate status

The archive records `/dev/kvm` absent. A separate explicit QEMU `-accel kvm`
initialization exited 1 with “Could not access KVM kernel module”; no KVM guest
started. The external runner returned 3 with a **BLOCKED** receipt. The
verifier returned 1 because the receipt is blocked and contains no guest proof;
that is not a failed KVM guest trial.

No hardware-backed `#XF` trial or official strict T2/T3/T4 gate completed.
The diagnostic 120/120 aggregate and diagnostic T2/T3/T4 results remain
separate evidence. Phase 15 stays **PARTIAL / UNQUALIFIED**. The next required
execution is the pinned external kit on an authorized x86-64 host with a
working KVM device (or Windows/WHPX), followed by the official strict gates if
the genuine vector-19 test passes.
