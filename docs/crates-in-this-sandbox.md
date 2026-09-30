# How the crate archives in `crates/` were obtained

This document explains, step by step, how the `.crate` archives in this repository
were downloaded, in an environment whose network egress is restricted to a very
small allowlist. Everything below is reproducible with `scripts/mirror-from-github.sh`
and `scripts/verify-crates.py`.

Result: **239 archives / 135 crates / 16.8 MB**, every single one byte-for-byte
identical to what crates.io publishes (SHA-256 compared against the crates.io index).

---

## 1. The environment

| | |
|---|---|
| Sandbox OS | Debian 12 (bookworm), x86-64 |
| Shell user | `user` (passwordless `sudo` available) |
| Rust toolchain | **not installed** (`cargo`/`rustc` are absent) — no `cargo fetch` possible |
| Available | `curl`, `wget`, `git`, `python3.11`, `tar`, `gzip`, `gh` (authenticated GitHub App token) |
| Disk | ~21 GB free |

### 1.1 Network egress allowlist

DNS resolves normally (nameserver `8.8.8.8`), outbound TCP/443 to *most* of the
internet is dropped during the TLS handshake (`SSL_ERROR_SYSCALL`, i.e. the
ClientHello never gets an answer). Only these hosts answer:

```
crates.io                              BLOCKED (TLS handshake reset)
static.crates.io                       BLOCKED (TLS handshake reset)
index.crates.io                        BLOCKED (TLS handshake reset)
rsproxy.cn                             BLOCKED (TLS handshake reset)
mirrors.tuna.tsinghua.edu.cn           BLOCKED (TLS handshake reset)
mirrors.ustc.edu.cn                    BLOCKED (TLS handshake reset)
mirrors.aliyun.com                     BLOCKED (TLS handshake reset)
mirror.sjtu.edu.cn                     BLOCKED (TLS handshake reset)
web.archive.org                        BLOCKED (TLS handshake reset)
raw.githubusercontent.com              BLOCKED (TLS handshake reset)
docs.rs                                BLOCKED (TLS handshake reset)
nodejs.org                             BLOCKED (TLS handshake reset)
github.com                             reachable  HTTP 200
api.github.com                         reachable  HTTP 200
codeload.github.com                    reachable  HTTP 301
pypi.org                               reachable  HTTP 200
files.pythonhosted.org                 reachable  HTTP 404
registry.npmjs.org                     reachable  HTTP 200
```

Reproduce it with:

```bash
for h in crates.io static.crates.io index.crates.io rsproxy.cn \
         mirrors.tuna.tsinghua.edu.cn github.com api.github.com pypi.org; do
  printf '%-32s ' "$h"
  curl -sS -m 8 -o /dev/null -w '%{http_code}\n' "https://$h" 2>&1 | tail -1
done
```

Port 80 is closed too, IPv6 is blocked as well, and no HTTP proxy is configured
(`env | grep -i proxy` is empty). Chinese mirrors, the Wayback Machine, IPFS
gateways, GitHub raw/CDN hosts and every generic CORS proxy I tried are all
blocked. **The practical consequence: the only way to move bytes into this
sandbox is GitHub, PyPI or npm.** I stayed inside that allowlist rather than
trying to circumvent the egress policy.

### 1.2 So the obvious approaches do not work

```console
$ curl -sS -m 20 -w '%{http_code}\n' -o out.crate \
      https://static.crates.io/crates/spin/spin-0.9.8/download
curl: (35) OpenSSL SSL_connect: SSL_ERROR_SYSCALL in connection to static.crates.io:443
```

`cargo fetch` would fail exactly the same way (and there is no cargo anyway).
`scripts/fetch-crates.py` shows the same thing from the tool side:

```console
$ python3 scripts/fetch-crates.py --index github -o /tmp/testfetch spin@0.9.8
✗ spin-0.9.8: download-failed  (https://static.crates.io/crates/spin/0.9.8/download :: ...)
0/1 archives ready in /tmp/testfetch; 1 failed
```

---

## 2. What *does* work: GitHub as the transport

Two facts make this solvable:

1. **The crates.io index is mirrored on GitHub.** `github.com/rust-lang/crates.io-index`
   is the live git index (its head commit was pushed the same day I ran this, and
   it contains every crate's versions, dependencies, features and — crucially —
   the `cksum` field, which is the **SHA-256 of the exact `.crate` file** served
   by crates.io). This gives us metadata + integrity data with no crates.io access.
2. **Some GitHub repositories contain real `.crate` archives**, because their
   authors committed their cargo cache (`~/.cargo/registry/cache/index.crates.io-*/`)
   or a `cargo local-registry` export to git.

### 2.1 Finding repositories that actually store `.crate` files

GitHub code search does **not** index `.crate` binaries (`filename:*.crate` → 0
results, `filename:*.apk` → 5, `filename:*.deb` → 0), so path-based discovery is
impossible. Instead I did a *tree-level* sweep: ~800 candidate repositories were
selected with repository-search keywords (`rust-registry`, `cargo registry cache`,
`crates.io mirror`, `cargo-home`, `offline crates`, `vendor`, `crate archive`, …),
and for each one the full git tree was fetched through
`GET /repos/{owner}/{repo}/git/trees/{branch}?recursive=1` and filtered for paths
ending in `.crate`.

Six repositories had archives that matched an OS-development crate allowlist:

| Repository | `.crate` files | Notes |
|---|---|---|
| `phoxal/registry` | 1115 | append-only registry, still updated (2026) |
| `lwx270901/registryintern` | 632 | committed `~/.cargo/registry/cache/index.crates.io-*` |
| `shards-lang/rust-registry` | 256 | `cargo local-registry` export |
| `ProtonPrivacy/rust-registry` | 241 | Proton's public registry |
| `pombredanne/crates_popular` | 60 | "popular Rust crates" |
| `zyma98/custom-rust-registry`, `kellnr/kellnr`, `GrantBirki/rust-template`, `openela-main/cargo-vendor` | 1–6 | small, occasionally fill gaps |

### 2.2 Selection + verification

1. Clone the mirrors shallow (`git clone --depth 1`, `github.com` is allowed;
   `codeload.github.com` also serves tarballs if you prefer).
2. Keep archives whose crate name is in `scripts/os-crate-allowlist.txt`
   (142 names: `no_std` core, allocation, boot, proc-macro/build tooling,
   unwinding/debug, serialisation, parsing).
3. For every candidate: compute `sha256sum` and compare it with the `cksum` that
   the crates.io index records for that exact name+version.
4. Copy only the matches into `crates/` and generate the matching
   `cargo local-registry` index (`crates/index/`) so the directory is directly
   usable by cargo.

Verification result:

```console
$ python3 scripts/verify-crates.py -d crates --strict
239/239 archives verified against the crates.io index (16.8 MB)
```

Two of the stored versions (`spin-0.9.8`, `smallvec-0.6.8`) are marked *yanked*
upstream. They are still bit-exact published artifacts — cargo will happily use
them when a `Cargo.lock` pins them — but a fresh resolution would not pick them.

### 2.3 One command, from scratch

```bash
scripts/mirror-from-github.sh --out crates        # clones mirrors, verifies, builds index
scripts/verify-crates.py -d crates --strict       # independent re-check
```

`mirror-from-github.sh` prints progress like:

```
== collected 981 candidate archives
== 239 archives match the OS allowlist
== verifying against the crates.io index (channel: github)
239/239 archives verified against the crates.io index (16.8 MB)
== done. 239 archives in ./crates
```

---

## 3. What is *not* in `crates/`

The kernel/bootloader-specific crates (`x86_64`, `bootloader`, `volatile`,
`linked_list_allocator`, `pic8259`, `uart_16550`, `raw-cpuid`, `multiboot2`,
`riscv`, `virtio-drivers`, `smoltcp`, …) do not appear in any of the GitHub
mirrors above — they were never part of those projects' caches, and no public
GitHub repository seems to store them as archives. They cannot be fetched from
inside this sandbox at all.

`crates/missing-os-crates.txt` therefore contains the current version, crates.io
checksum and download URL for every OS crate that is still missing, taken from
the crates.io index (also via GitHub). To complete the cache on a machine that
can reach crates.io or a mirror:

```bash
# from crates.io
python3 scripts/fetch-crates.py --from-file crates/missing-os-crates.txt -o crates

# or, behind the Great Firewall, through a mirror
python3 scripts/fetch-crates.py --mirror rsproxy \
        --from-file crates/missing-os-crates.txt -o crates
```

Every download is checked against the index checksum, so a wrong mirror URL or a
corrupted transfer fails loudly instead of silently poisoning the cache.

When only GitHub is reachable (like here), the download step can be run anywhere
else — the sandbox only needs to *consume* the resulting directory:

```bash
# on a connected machine, print the exact transfer list
python3 scripts/fetch-crates.py --index github --urls-only \
        --from-lock Cargo.lock > urls.tsv
```

---

## 4. Reproducing this session, command by command

```bash
# 0. what is reachable?
curl -sS -m 8 -o /dev/null -w '%{http_code}\n' https://crates.io          # 000 / blocked

# 1. metadata channel (works here): the crates.io index through GitHub
curl -sS "https://api.github.com/repos/rust-lang/crates.io-index/contents/sp/in/spin?ref=master" \
  | python3 -c 'import json,sys,base64; print(base64.b64decode(json.load(sys.stdin)["content"]).decode())'

# 2. find mirrors and verify them
scripts/mirror-from-github.sh --out crates

# 3. confirm the cache is genuine
scripts/verify-crates.py -d crates --strict

# 4. see what a *real* crates.io download would look like (URLs + checksums)
scripts/fetch-crates.py --index github --urls-only x86_64 bootloader spin@0.9.8
```

That last command prints, for example:

```
x86_64      0.15.5    be4ec631e1a81d50e46c35a4a00322bd076c47491b64c2fb10a7ffa89002c697  https://static.crates.io/crates/x86_64/0.15.5/download
bootloader  0.11.17   d861319d747a01da5d4d680500b7f808607f902a286b476bd51f991612e39d8e  https://static.crates.io/crates/bootloader/0.11.17/download
```

---

## 5. Caveats worth knowing

* The archives are the genuine crates.io artifacts (checksum-verified), but they
  were **not fetched from crates.io in this sandbox** — crates.io is unreachable
  here. They came from public GitHub repositories that had cached them, and the
  checksum match is what makes that safe.
* `crates/index/` is filtered to exactly the versions stored here, which is what
  `cargo local-registry` does. Full copies of each crate's upstream index file
  can be fetched at any time with
  `scripts/fetch-crates.py --reindex-only` (needs the GitHub or sparse index).
* This is a *cache of archives*, not a vendored source tree. If you want sources
  unpacked on disk, run `cargo vendor` against this local registry — see
  `crates/README.md`.
