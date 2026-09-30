# `crates/` — offline crates.io cache

**239 `.crate` archives, 135 crates, 16.8 MB.** Every archive is byte-for-byte the
artifact crates.io publishes (SHA-256 checked against the upstream index; see
`MANIFEST.tsv` and `../docs/crates-in-this-sandbox.md`).

```
crates/
├── *.crate                 the archives
├── SHA256SUMS              sha256 of every archive (GNU coreutils format)
├── MANIFEST.tsv            crate, version, bytes, sha256, mirror repo it came from
├── missing-os-crates.txt   OS crates NOT obtainable here: version + cksum + URL
└── index/                  cargo "local registry" index, filtered to the versions above
    ├── config.json
    └── sp/in/spin, x8/6_/x86_64, …      (same layout as crates.io's index)
```

## Verify it

```bash
scripts/verify-crates.py -d crates --strict      # uses the GitHub index mirror
sha256sum -c crates/SHA256SUMS                   # local check only
```

## Use it as a local registry (offline `cargo build`)

`crates/` has the exact layout `cargo local-registry` produces, so cargo can use
it as a replacement source. In the project that consumes it:

```toml
# .cargo/config.toml
[source.crates-io]
replace-with = "arenaos-local"

[source.arenaos-local]
local-registry = "/absolute/path/to/ArenaOS/crates"
```

Then `cargo build --offline` resolves against the archives in this directory.
Cargo only accepts versions that are present here — anything else must be added
first (see below), which is exactly the behaviour you want for an air-gapped box.

## Add the crates that are missing

The OS-specific crates (`x86_64`, `bootloader`, `volatile`,
`linked_list_allocator`, `pic8259`, `uart_16550`, `raw-cpuid`, `multiboot2`,
`riscv`, `virtio-drivers`, `smoltcp`, …) are unavailable inside this sandbox;
`missing-os-crates.txt` lists them with their current version, upstream SHA-256
and download URL. Run this on a machine that can reach crates.io (or a mirror):

```bash
# crates.io
python3 scripts/fetch-crates.py --from-file crates/missing-os-crates.txt -o crates

# or a mirror, e.g. rsproxy.cn / tuna / ustc / aliyun / sjtug
python3 scripts/fetch-crates.py --mirror rsproxy \
        --from-file crates/missing-os-crates.txt -o crates

# after which the index is refreshed and everything is re-verified
python3 scripts/verify-crates.py -d crates --strict
```

`--from-lock Cargo.toml`-generated `Cargo.lock` is also supported, which is the
usual workflow: build once with network access, then mirror the lock file.

## Turn it into a vendor directory (optional)

```bash
cargo vendor            # with the local-registry config above, writes vendor/
```

That unpacks sources and adds `.cargo-checksum.json` files, which is what
`[source.crates-io] replace-with = "vendored-sources"` expects. Keep the archives
around anyway: they are the transferable artifact, and they are what proves
provenance.
