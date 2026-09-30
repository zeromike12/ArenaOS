#!/usr/bin/env python3
"""fetch-crates.py - download crates.io `.crate` archives into a directory, with
SHA-256 verification against the crates.io index.

Why this exists: the sandbox this repo was prepared in can only reach github.com,
pypi.org and registry.npmjs.org. crates.io and every public crates.io mirror are
firewalled there. This script therefore supports three metadata channels and
several download channels, and always verifies what it downloaded.

Metadata (which versions exist, and their checksums):
  --index github   -> read the index through the GitHub mirror
                      github.com/rust-lang/crates.io-index (works in the sandbox)
  --index sparse   -> read the HTTP sparse index (index.crates.io or any mirror)
  --index auto     -> try sparse first, fall back to github      [default]

Downloads:
  --mirror crates-io|rsproxy|tuna|ustc|aliyun|sjtug|<url>
                   crates.io itself, or a mirror. The download base URL is taken
                   from the mirror's own index `config.json` when reachable, so
                   it is not a guess; the SHA-256 check catches any wrong URL.
  --dl-template    full manual control, e.g.
                   'https://static.crates.io/crates/{name}/{version}/download'

Examples
  ./fetch-crates.py spin bitflags@2.6.0
  ./fetch-crates.py --from-lock ../Cargo.lock
  ./fetch-crates.py --mirror rsproxy --from-lock ../Cargo.lock
  ./fetch-crates.py --index github --urls-only --from-file ../crates/missing-os-crates.txt
"""

from __future__ import annotations

import argparse
import base64
import functools
import hashlib
import json
import os
import re
import sys
import tarfile
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

GITHUB_INDEX_REPO = "rust-lang/crates.io-index"
GH_CONTENTS = f"https://api.github.com/repos/{GITHUB_INDEX_REPO}/contents/{{path}}?ref=master"
GH_BLOB = f"https://api.github.com/repos/{GITHUB_INDEX_REPO}/git/blobs/{{sha}}"
GH_DIR = f"https://api.github.com/repos/{GITHUB_INDEX_REPO}/contents/{{dir}}?ref=master"

SPARSE_DEFAULT = "https://index.crates.io/"

MIRRORS = {
    "crates-io": ("https://index.crates.io/", "https://static.crates.io/crates"),
    "rsproxy": ("https://rsproxy.cn/index/", "https://rsproxy.cn/api/v1/crates"),
    "tuna": ("https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/", None),
    "ustc": ("https://mirrors.ustc.edu.cn/crates.io-index/", None),
    "aliyun": ("https://mirrors.aliyun.com/crates.io-index/", None),
    "sjtug": ("https://mirror.sjtu.edu.cn/crates.io-index/", None),
}

UA = {"User-Agent": "arenaos-fetch-crates/1.0"}


def http_get(url: str, accept: str | None = None, timeout: int = 60) -> bytes:
    headers = dict(UA)
    if accept:
        headers["Accept"] = accept
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read()


def index_path(name: str) -> str:
    """crates.io index path for a crate name (1/<n>, 2/<n>, 3/<c>/<n>, ab/cd/<n>)."""
    n = name.lower()
    if len(n) == 1:
        return "1/" + n
    if len(n) == 2:
        return "2/" + n
    if len(n) == 3:
        return "3/" + n[0] + "/" + n
    return n[0:2] + "/" + n[2:4] + "/" + n


# --------------------------------------------------------------------------- #
# metadata channels
# --------------------------------------------------------------------------- #
@functools.lru_cache(maxsize=None)
def index_rows_github(name: str) -> list[dict]:
    path = index_path(name)
    try:
        meta = json.loads(http_get(GH_CONTENTS.format(path=path), "application/vnd.github+json"))
        if meta.get("content"):
            raw = base64.b64decode(meta["content"])
        else:
            # > 1 MiB index files come back without inline content: resolve the blob.
            d, f = path.rsplit("/", 1)
            listing = json.loads(http_get(GH_DIR.format(dir=d), "application/vnd.github+json"))
            sha = next(x["sha"] for x in listing if x["name"] == f)
            raw = http_get(GH_BLOB.format(sha=sha), "application/vnd.github.raw")
    except (urllib.error.URLError, KeyError, StopIteration, ValueError) as exc:
        raise RuntimeError(f"github index lookup failed for {name}: {exc}") from exc
    return [json.loads(line) for line in raw.decode().splitlines() if line.strip()]


@functools.lru_cache(maxsize=None)
def index_rows_sparse(name: str, base: str) -> list[dict]:
    raw = http_get(base.rstrip("/") + "/" + index_path(name))
    return [json.loads(line) for line in raw.decode().splitlines() if line.strip()]


def index_config_sparse(base: str) -> dict:
    try:
        return json.loads(http_get(base.rstrip("/") + "/config.json"))
    except Exception:  # noqa: BLE001
        return {}


class Index:
    def __init__(self, mode: str, sparse_base: str):
        self.mode = mode
        self.sparse_base = sparse_base
        self.mode_used: str | None = None
        self.warned = False

    def rows(self, name: str) -> list[dict]:
        if self.mode in ("auto", "sparse"):
            try:
                rows = index_rows_sparse(name, self.sparse_base)
                self.mode_used = self.mode_used or "sparse"
                return rows
            except Exception:  # noqa: BLE001
                if self.mode == "sparse":
                    raise
                if not self.warned:
                    self.warned = True
                    print("  · sparse index unreachable, using the GitHub index mirror", file=sys.stderr)
        rows = index_rows_github(name)
        self.mode_used = "github"
        return rows


# --------------------------------------------------------------------------- #
# version selection
# --------------------------------------------------------------------------- #
def version_key(v: str) -> tuple:
    core = re.split(r"[-+]", v, 1)[0]
    parts = [int(x) if x.isdigit() else 0 for x in core.split(".")]
    return tuple(parts + [0] * (3 - len(parts)))


def pick(rows: list[dict], want: str | None) -> dict:
    if want:
        for r in rows:
            if r["vers"] == want:
                return r
        raise SystemExit(f"version {want} not found in the index")
    live = [r for r in rows if not r.get("yanked")]
    stable = [r for r in live if re.match(r"^\d+(\.\d+)*$", r["vers"].split("+")[0])]
    return max(stable or live, key=lambda r: version_key(r["vers"]))


# --------------------------------------------------------------------------- #
# parsing inputs
# --------------------------------------------------------------------------- #
def parse_lock(path: Path) -> list[str]:
    """Pull name@version out of a Cargo.lock (only registry packages)."""
    specs, name, vers = [], None, None
    for line in path.read_text().splitlines():
        s = line.strip()
        if s == "[[package]]":
            if name and vers:
                specs.append(f"{name}@{vers}")
            name = vers = None
        elif s.startswith("name = "):
            name = s.split("=", 1)[1].strip().strip('"')
        elif s.startswith("version = "):
            vers = s.split("=", 1)[1].strip().strip('"')
    if name and vers:
        specs.append(f"{name}@{vers}")
    return specs


def parse_spec_file(path: Path) -> list[str]:
    """Accept 'name', 'name@version', 'name version', or tab-separated tables."""
    out = []
    for line in path.read_text().splitlines():
        line = line.split("#", 1)[0].rstrip()
        if not line.strip():
            continue
        if "\t" in line:                      # table row: name<TAB>version<TAB>...
            out.append(line.split("\t")[0].strip())
            continue
        parts = line.split()
        if len(parts) >= 2 and parts[1][:1].isdigit():
            out.append(f"{parts[0]}@{parts[1]}")
        else:
            out.append(parts[0])
    return out


# --------------------------------------------------------------------------- #
# download + verify
# --------------------------------------------------------------------------- #
def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def archive_root_ok(path: Path, name: str, version: str) -> bool:
    try:
        with tarfile.open(path, "r:gz") as tf:
            names = tf.getnames()
        prefix = f"{name}-{version}/"
        return bool(names) and all(n == names[0] or n.startswith(prefix) for n in names) and names[0].startswith(prefix)
    except Exception:  # noqa: BLE001
        return False


def dl_url(template: str, name: str, version: str) -> str:
    return template.format(name=name, version=version, crate=f"{name}-{version}.crate")


def main() -> int:
    ap = argparse.ArgumentParser(description="Download crates.io archives with checksum verification")
    ap.add_argument("specs", nargs="*", help="crate or crate@version")
    ap.add_argument("--from-lock", type=Path, help="read name@version pairs from a Cargo.lock")
    ap.add_argument("--from-file", type=Path, help="read 'name', 'name@version' or 'name version' lines")
    ap.add_argument("--index", choices=["auto", "sparse", "github"], default="auto")
    ap.add_argument("--index-url", default=SPARSE_DEFAULT, help="sparse index base URL")
    ap.add_argument("--mirror", default="crates-io", help="crates-io | rsproxy | tuna | ustc | aliyun | sjtug | URL")
    ap.add_argument("--dl-template", help="full download URL template, {name} {version} {crate}")
    ap.add_argument("-o", "--out", type=Path, default=Path("crates"))
    ap.add_argument("-j", "--jobs", type=int, default=4)
    ap.add_argument("--urls-only", action="store_true", help="print url + sha256 instead of downloading")
    ap.add_argument("--force", action="store_true", help="re-download even if the file exists")
    ap.add_argument("--no-index", action="store_true", help="do not write local-registry index entries")
    ap.add_argument("--reindex-only", action="store_true",
                    help="skip downloads; rebuild index/ and SHA256SUMS from the archives already in --out")
    args = ap.parse_args()

    if args.reindex_only:
        idx = Index(args.index, args.index_url)
        have = []
        for f in sorted(args.out.glob("*.crate")):
            m = re.match(r"^(?P<name>.+?)[-@](?P<ver>\d+(?:\.\d+)*(?:[-+][0-9A-Za-z.+-]*)?)$", f.stem)
            if m:
                have.append((m["name"], m["ver"]))
        write_index(args.out, have, idx)
        write_manifest(args.out)
        print(f"reindexed {len(have)} archives in {args.out}")
        return 0

    specs = list(args.specs)
    if args.from_lock:
        specs += parse_lock(args.from_lock)
    if args.from_file:
        specs += parse_spec_file(args.from_file)
    specs = list(dict.fromkeys(specs))
    if not specs:
        ap.error("no crates given (pass specs, --from-lock or --from-file)")

    mirror = args.mirror
    if mirror in MIRRORS:
        mirror_index, mirror_dl = MIRRORS[mirror]
        if args.index == "auto" and mirror_index and not args.index_url:
            args.index_url = mirror_index
    else:
        mirror_dl = None  # a raw URL was given

    idx = Index(args.index, args.index_url)

    # pick the download template
    dl = args.dl_template
    if not dl and mirror not in MIRRORS:
        dl = mirror  # user passed a raw template/URL
    if not dl:
        cfg = index_config_sparse(mirror_index if mirror in MIRRORS else args.index_url)
        base = cfg.get("dl") or mirror_dl
        if not base:
            print(":: warning: could not discover the mirror's download base URL.", file=sys.stderr)
            print("            pass --dl-template instead, e.g.", file=sys.stderr)
            print("            --dl-template 'https://static.crates.io/crates/{name}/{version}/download'", file=sys.stderr)
            return 2
        dl = base.rstrip("/") + "/{name}/{version}/download"

    args.out.mkdir(parents=True, exist_ok=True)
    plan, results = [], []

    for spec in specs:
        name, _, want = spec.partition("@")
        try:
            entry = pick(idx.rows(name), want or None)
        except (RuntimeError, SystemExit) as exc:
            results.append((name, want or "?", "error", str(exc)))
            continue
        url = dl_url(dl, entry["name"], entry["vers"])
        plan.append((entry["name"], entry["vers"], entry["cksum"], url))

    if args.urls_only:
        print("# crate\tversion\tsha256\turl")
        for name, ver, cks, url in plan:
            print(f"{name}\t{ver}\t{cks}\t{url}")
        return 0

    def worker(item):
        name, ver, cks, url = item
        out = args.out / f"{name}-{ver}.crate"
        if out.exists() and not args.force and sha256(out) == cks:
            return (name, ver, "cached", url)
        tmp = out.with_suffix(".crate.part")
        try:
            data = http_get(url, timeout=300)
        except Exception as exc:  # noqa: BLE001
            return (name, ver, "download-failed", f"{url} :: {exc}")
        tmp.write_bytes(data)
        got = sha256(tmp)
        if got != cks:
            tmp.unlink(missing_ok=True)
            return (name, ver, "checksum-mismatch", f"index={cks[:16]}… got={got[:16]}… url={url}")
        if not archive_root_ok(tmp, name, ver):
            tmp.unlink(missing_ok=True)
            return (name, ver, "bad-archive", url)
        tmp.replace(out)
        return (name, ver, "ok", url)

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for res in pool.map(worker, plan):
            results.append(res)
            mark = {"ok": "✓", "cached": "="}.get(res[2], "✗")
            detail = "" if res[2] in ("ok", "cached") else f"  ({res[3]})"
            print(f"{mark} {res[0]}-{res[1]}: {res[2]}{detail}")

    ok = [r for r in results if r[2] in ("ok", "cached")]
    if not args.no_index:
        write_index(args.out, [(r[0], r[1]) for r in ok], idx)
    write_manifest(args.out)
    bad = [r for r in results if r[2] not in ("ok", "cached")]
    print(f"\n{len(ok)}/{len(results)} archives ready in {args.out}"
          + (f"; {len(bad)} failed" if bad else ""))
    return 1 if bad else 0


# --------------------------------------------------------------------------- #
# local registry bookkeeping
# --------------------------------------------------------------------------- #
def write_index(out: Path, have: list[tuple[str, str]], idx: Index) -> None:
    """Write a `cargo local-registry` style index containing only stored versions."""
    cfg = {"dl": "https://static.crates.io/crates", "api": "https://crates.io"}
    (out / "index").mkdir(exist_ok=True)
    (out / "index" / "config.json").write_text(json.dumps(cfg, indent=2) + "\n")
    by_name: dict[str, set] = {}
    for name, ver in have:
        by_name.setdefault(name, set()).add(ver)
    for name, versions in sorted(by_name.items()):
        try:
            rows = idx.rows(name)
        except Exception:  # noqa: BLE001
            continue
        keep = [json.dumps(r, separators=(",", ":")) for r in rows if r["vers"] in versions]
        dst = out / "index" / index_path(name)
        dst.parent.mkdir(parents=True, exist_ok=True)
        existing = dst.read_text().splitlines() if dst.exists() else []
        merged = {json.loads(l)["vers"]: l for l in existing if l.strip()}
        merged.update({json.loads(l)["vers"]: l for l in keep})
        dst.write_text("\n".join(merged[v] for v in sorted(merged, key=version_key)) + "\n")


def write_manifest(out: Path) -> None:
    rows = []
    for f in sorted(out.glob("*.crate")):
        if not re.match(r"^.+?[-@]\d+(?:\.\d+)*(?:[-+][0-9A-Za-z.+-]*)?$", f.stem):
            continue
        rows.append(f"{sha256(f)}  {f.name}")
    (out / "SHA256SUMS").write_text("\n".join(rows) + "\n")
    print(f"wrote {out/'SHA256SUMS'} ({len(rows)} archives)")


if __name__ == "__main__":
    sys.exit(main())
