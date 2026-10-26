# Phase 16 dependency-transfer request (B1) — for M/C

**Resolved 2026-10-26:** C supplied the numbered parts in commit
`02a20fce915bdd0bdbbc24dcdc2fa0b63a1cd7a1` under
`docs/phase16/dependencies/rust-1.97.0/`. All parts matched
`PARTS-SHA256SUMS`; both reconstructed archives matched `SHA256SUMS`;
the official `install.sh` scripts installed them into the private
`/opt/rust/prefix`; the protected Phase 14 PIE/APB1 reproduced and the
canonical battery ran (see `receipts/canonical-2026-10-26/`). The
original request below is retained as history.

**One ask (historical):** supply these two unmodified official Rust 1.97.0 distribution
archives (any channel: attached to the repo as release assets, a branch
transfer commit of the raw files, or another sandbox-reachable copy with
identical bytes). Everything else in Phase 16 is unblocked.

| Archive | URL (upstream) | Expected SHA-256 |
|---|---|---|
| `rust-std-1.97.0-x86_64-unknown-none.tar.xz` | `https://static.rust-lang.org/dist/2026-07-09/rust-std-1.97.0-x86_64-unknown-none.tar.xz` | `5b6702d1ce758d4d01fc3861085bbf1ee19eb9d7f7dc69e066c299ee9606168a` |
| `rust-std-1.97.0-x86_64-unknown-uefi.tar.xz` | `https://static.rust-lang.org/dist/2026-07-09/rust-std-1.97.0-x86_64-unknown-uefi.tar.xz` | `2c48c8b659d65ec74e916e44e5ecac8e77f4c1abce51d01d661f133233641ad0` |

(channel manifest dated 2026-07-09; hashes cross-checked in
`docs/phase15/receipts/phase14-pie-recovery.md`.)

**Failed retrieval commands (sandbox egress allowlist):**

- `curl -LO https://static.rust-lang.org/dist/2026-07-09/rust-std-1.97.0-x86_64-unknown-none.tar.xz`
  → TLS connection reset (`static.rust-lang.org` not allowlisted).
- `rustup target add x86_64-unknown-none x86_64-unknown-uefi` → same host
  failure.
- GitHub (`api.github.com` / `codeload.github.com`) search for an official
  mirror → none exists; Rust publishes these components only via
  `static.rust-lang.org`. PyPI/npm have no official rust-std components.

**On receipt I will:** verify SHA-256 before installation, install into
the toolchain prefix, confirm the protected Phase 14 PIE rebuild produces
`d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e` via the
unchanged `tools/build_phase14_pie.sh` guard, then run the canonical
qualification battery. Build guards and expected hashes stay untouched
either way; development-image results remain labeled DEVELOPMENT until
then (`docs/phase16/RECOVERY-LOG.md` has the full attempt history).
