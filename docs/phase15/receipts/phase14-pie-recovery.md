# Phase 14 PIE reproducibility recovery

The Phase 14 guard remains unchanged at
`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e`.
`userspace/phase14-pie` source, lockfile, linker script, and local Rust flags
match the protected fixture's committed inputs.

The compiler-version match had hidden a target-sysroot mismatch. The previous
workspace environment compiled `x86_64-unknown-none` libraries from the Rust
1.97.0 source tree with `tools/build-sysroot.sh`. Phase 14's qualification
record says its guest targets came from official `rustup` components. The
`rustc` and `rust-lld` executable SHA-256 values are identical between the
workspace install and the official `arena-rust-toolchain==1.97.0` PyPI
package; the target libraries are not the same build input.

The canonical build used these executable identities:

- `rustc`: `df13f58759c0662831983e3a6501c63c1fc12ea60ec4e1d1ac35e5fe43c500c0`
- `rust-lld`: `8e5130a02d8f3473ed4c4b971d43c301205c89854bb467257e64eafe4bf21ca8`

Clean release builds with the two target sysroots establish the cause:

| Target standard library | Resulting ELF SHA-256 |
|---|---|
| Workspace source-built `x86_64-unknown-none` | `2c3720dd4299dba89f95f78f16ca21fbd5a2f3a715a05ede6484a7d189c9b6dd` |
| Official Rust 1.97.0 `rust-std-x86_64-unknown-none` | `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e` |

The official target component URLs and expected SHA-256 values are in the
Rust 1.97.0 channel manifest (`2026-07-09`):

- `x86_64-unknown-none`: `5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a`
- `x86_64-unknown-uefi`: `2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0`

Both downloaded archives matched the manifest. Installing these official
target components into the exact Rust 1.97.0 toolchain restored the PIE
identity. `tools/build_phase14_pie.sh` then passed without edits and rebuilt
the pinned signed package at its existing digest
`04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa`.

The first full build after changing sysroots reused Cargo target directories
whose dependency artifacts referenced the source-built core libraries. Rust
rejected those stale artifacts with E0463. Removing generated `target/`
directories and running the normal command again made it a clean build;
`tools/build.sh --image` then completed, including all historical package
verification, the kernel ISA audit, and ESP creation. No hash guard, signature
check, loader validation, or expected digest was changed.
