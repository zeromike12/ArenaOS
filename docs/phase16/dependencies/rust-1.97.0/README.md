# Official Rust 1.97.0 target components for Phase 16

These unmodified official distribution archives supply the components requested in `../../DEPENDENCY-REQUEST.md`. C downloaded them directly from Rust's official host on 2026-10-25 and verified both archive SHA-256 values against the recorded Phase 15/16 recovery identities.

They are stored as five numbered binary parts because the GitHub connector has a 16 MiB request limit. Concatenation restores the original archive bytes exactly; no executable wrapper or custom installer is required.

## Reassemble and verify

From this directory:

```sh
sha256sum --check PARTS-SHA256SUMS
cat rust-std-1.97.0-x86_64-unknown-none.tar.xz.part-* > rust-std-1.97.0-x86_64-unknown-none.tar.xz
cat rust-std-1.97.0-x86_64-unknown-uefi.tar.xz.part-* > rust-std-1.97.0-x86_64-unknown-uefi.tar.xz
sha256sum --check SHA256SUMS
```

The reconstructed `.tar.xz` archives are generated local copies; do not commit duplicate copies of them. Extract each archive to a temporary directory and use its official `install.sh` with the matching private Rust 1.97.0 toolchain prefix. Determine the actual prefix before installing. Do not replace host system tools.

## Provenance and exact archive identities

- Source: https://static.rust-lang.org/dist/2026-07-09/rust-std-1.97.0-x86_64-unknown-none.tar.xz
  - Bytes: 14,907,620
  - SHA-256: `5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a`
- Source: https://static.rust-lang.org/dist/2026-07-09/rust-std-1.97.0-x86_64-unknown-uefi.tar.xz
  - Bytes: 25,374,412
  - SHA-256: `2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0`

Archive paths and supplied install scripts were inspected; both archives remain unchanged. `PARTS-SHA256SUMS` verifies every stored part and `SHA256SUMS` verifies the reconstructed archives.

## Resume canonical qualification

After installation, clear only generated Cargo artifacts that depend on the old sysroot if needed. Run the unchanged protected Phase 14 rebuild and canonical `tools/build.sh` workflow. Required protected identities remain:

- PIE: `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`
- Signed APB1: `04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`

Record whether B1 is resolved. Complete canonical Phase 16 guest tests, the outstanding two-candidate Notification ambiguity negative, full registered regressions, required final-artifact stability including 100/100, fresh-archive verification, demonstration package, and Arena 1 handoff.

This commit supplies dependencies only. It changes no OS source, build guard, expected digest, qualification receipt, or architecture decision. Earlier development evidence keeps its classification. No merge, release, Phase 16 acceptance, or Phase 17 work is authorized by this handoff.
