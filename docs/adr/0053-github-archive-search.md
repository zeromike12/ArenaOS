# ADR-0053 — GitHub archive-cache acquisition search (2026-09-30)

**Result: 1/23 exact candidate `.crate` archives recovered and SHA-256 verified; 22 remain unavailable. This is NOT an accepted dependency audit or permission to implement the guest verifier.** No version substitution was made.

## Method and scope

Inspected the separate `arena/01a0f3fc-arenaos` branch **without checking it out** (fetched at `d0589a1`); read its `docs/crates-in-this-sandbox.md`, `crates/MANIFEST.tsv`, and `scripts/mirror-from-github.sh`/`verify-crates.py`. Reused the *discovery method*: look for committed `.crate` binaries in GitHub cargo caches/local registries using repository search and GitHub's recursive Git-tree API. GitHub's code search does not index these binary archives. We did **not** reuse its old dependency choices or rely on its manifest as authentication.

Parsed the 23 exact names/versions and registry SHA-256s from our own ADR-0053. Searched repository metadata using 41 queries across `rust-registry`, `cargo registry cache`, `crates.io mirror`, `cargo local-registry`, `offline crates`, `cargo-home`, `rust vendor`, `.crate archive`, `crates.io-index`, plus recent `rust registry`, `cargo registry`, `ed25519-dalek crate`, and related search results. Collected **1,284 distinct candidate repositories** (nine known cache/mirror candidates plus 1,275 additional Git-tree probes); **1,222** complete tree/empty results, **five truncated recursive trees** (primarily crate *source* vendor/index repositories), and **57 API errors** (one 404; the final 56 were GitHub installation API rate-limit 403s). This was a broad candidate-repository search, **not a proof that no unindexed GitHub repository contains the archives**. Avoid more API requests until the limit resets.

The known archive caches (`phoxal/registry`, `lwx270901/registryintern`, `shards-lang/rust-registry`, `ProtonPrivacy/rust-registry`, `pombredanne/crates_popular` and several small repos) contained no other exact candidate. A few additional committed-archive repositories found by the tree probes (`clipos/assets_crates-io`, `UMM-CSci-4553-S25/registry`, `fluxcodestudio/crate-format`, `YurtOS/yurt-crates` and others) likewise contained none. A matching **source directory** in a vendor repo is not an authenticated published `.crate` archive.

The only matching published archive in the other ArenaOS branch is `crates/version_check-0.9.5.crate` (15,554 bytes). Independently streamed the exact Git blob to ignored `build/phase84-crates/` and checked with our `tools/package_vendor.py` checksum lookup: SHA-256 **`0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a`**, identical to the registry-index candidate checksum in ADR-0053. This ignored local file is **not** a completed vendor tree; the 23-archive vendor tool deliberately refuses to create a partial destination.

## Exact missing archives (22/23)

| | | |
|---|---|---|
| `block-buffer-0.10.4.crate` | `cfg-if-1.0.5.crate` | `cpufeatures-0.2.17.crate` |
| `crypto-common-0.1.7.crate` | `curve25519-dalek-4.1.3.crate` | `curve25519-dalek-derive-0.1.1.crate` |
| `digest-0.10.7.crate` | `ed25519-2.2.3.crate` | `ed25519-dalek-2.2.0.crate` |
| `fiat-crypto-0.2.9.crate` | `generic-array-0.14.7.crate` | `libc-0.2.189.crate` |
| `proc-macro2-1.0.107.crate` | `quote-1.0.47.crate` | `rustc_version-0.4.1.crate` |
| `semver-1.0.28.crate` | `sha2-0.10.9.crate` | `signature-2.2.0.crate` |
| `subtle-2.6.1.crate` | `syn-2.0.119.crate` | `typenum-1.20.1.crate` |
| `unicode-ident-1.0.26.crate` | — | — |

**Next step:** obtain these **same exact archives** from a connected machine/mirror or a newly located GitHub cache; verify each against its ADR-0053 index checksum. Once all 23 exist in one directory, run `python3 tools/package_vendor.py --archives <dir> --out build/phase84-sources`. That validates/unpacks them but does **not** replace the license/provenance, feature, unsafe/build/proc-macro, independent verifier/vector and offline target build gates. Do not use older cache versions to make the set look complete.
