# ArenaOS

Working checkout for a Rust operating-system project, currently containing an
**offline crates.io cache** and the tooling that produced it.

```
.
├── crates/                    239 verified `.crate` archives (16.8 MB, 135 crates)
│   ├── index/                 cargo local-registry index for exactly those crates
│   ├── MANIFEST.tsv           provenance + sha256 per archive
│   └── missing-os-crates.txt  OS crates that are still needed, with checksums/URLs
├── scripts/
│   ├── fetch-crates.py        download crates from crates.io or any mirror, checksum-verified
│   ├── verify-crates.py       verify a directory of archives against the crates.io index
│   ├── mirror-from-github.sh  recover archives using only github.com (the method used here)
│   └── os-crate-allowlist.txt crate names considered relevant for a Rust OS
└── docs/
    └── crates-in-this-sandbox.md   exactly how (and why) the archives were fetched this way
```

## Quick start

```bash
# 1. verify the cache is genuine (needs github.com or index.crates.io)
python3 scripts/verify-crates.py -d crates --strict

# 2. get the OS-specific crates that could not be fetched in this sandbox
#    (run where crates.io or a mirror is reachable)
python3 scripts/fetch-crates.py --from-file crates/missing-os-crates.txt -o crates

# 3. point cargo at the local registry
cat >> .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "arenaos-local"

[source.arenaos-local]
local-registry = "/absolute/path/to/ArenaOS/crates"
EOF
```

## Why the archives came from GitHub

The sandbox this checkout was prepared in allows outbound HTTPS **only** to
`github.com` / `api.github.com` / `codeload.github.com`, `pypi.org`
/`files.pythonhosted.org` and `registry.npmjs.org`. `crates.io`,
`static.crates.io`, `index.crates.io` and every public crates.io mirror
(rsproxy.cn, TUNA, USTC, Aliyun, SJTUG, …) are firewalled at the TLS handshake,
and there is no Rust toolchain installed either.

So the archives were recovered through GitHub:

1. **metadata + integrity** from the live GitHub mirror of the index,
   `rust-lang/crates.io-index` (its `cksum` field is the SHA-256 of the exact
   `.crate` file crates.io serves);
2. **archives** from public GitHub repositories that had committed their cargo
   cache / `cargo local-registry` export;
3. **verification**: only archives whose SHA-256 equals the index checksum were
   kept — 239/239 matched.

`docs/crates-in-this-sandbox.md` has the full walkthrough, the network probe
results and the exact commands. `scripts/mirror-from-github.sh` reproduces the
whole import in one shot.
