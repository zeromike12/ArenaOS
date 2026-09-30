#!/usr/bin/env python3
"""verify-crates.py - check every `.crate` in a directory against

  1. the recorded SHA256SUMS file next to it, and
  2. the upstream crates.io index checksum (`cksum` field), which is the SHA-256
     of the exact archive crates.io serves.

Channel 2 is what makes an offline cache trustworthy: it proves the file is the
byte-for-byte artifact published on crates.io, without downloading it again.

  ./verify-crates.py                       # verify ./crates
  ./verify-crates.py -d crates --strict    # non-zero exit on any failure
  ./verify-crates.py --index sparse        # use index.crates.io instead of GitHub
"""

from __future__ import annotations

import argparse
import base64
import functools
import hashlib
import json
import re
import sys
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

GH_CONTENTS = "https://api.github.com/repos/rust-lang/crates.io-index/contents/{path}?ref=master"
GH_BLOB = "https://api.github.com/repos/rust-lang/crates.io-index/git/blobs/{sha}"
GH_DIR = "https://api.github.com/repos/rust-lang/crates.io-index/contents/{dir}?ref=master"
UA = {"User-Agent": "arenaos-verify-crates/1.0"}
NAME_RE = re.compile(r"^(?P<name>.+?)[-@](?P<ver>\d+(?:\.\d+)*(?:[-+][0-9A-Za-z.+-]*)?)$")


def http_get(url: str, accept: str | None = None, timeout: int = 60) -> bytes:
    headers = dict(UA)
    if accept:
        headers["Accept"] = accept
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=timeout) as r:
        return r.read()


def index_path(name: str) -> str:
    n = name.lower()
    if len(n) == 1:
        return "1/" + n
    if len(n) == 2:
        return "2/" + n
    if len(n) == 3:
        return "3/" + n[0] + "/" + n
    return n[0:2] + "/" + n[2:4] + "/" + n


@functools.lru_cache(maxsize=None)
def rows_github(name: str) -> list[dict]:
    path = index_path(name)
    meta = json.loads(http_get(GH_CONTENTS.format(path=path), "application/vnd.github+json"))
    if meta.get("content"):
        raw = base64.b64decode(meta["content"])
    else:
        d, f = path.rsplit("/", 1)
        listing = json.loads(http_get(GH_DIR.format(dir=d), "application/vnd.github+json"))
        sha = next(x["sha"] for x in listing if x["name"] == f)
        raw = http_get(GH_BLOB.format(sha=sha), "application/vnd.github.raw")
    return [json.loads(line) for line in raw.decode().splitlines() if line.strip()]


@functools.lru_cache(maxsize=None)
def rows_sparse(name: str, base: str) -> list[dict]:
    raw = http_get(base.rstrip("/") + "/" + index_path(name))
    return [json.loads(line) for line in raw.decode().splitlines() if line.strip()]


def cksum_for(name: str, version: str, mode: str, sparse_base: str) -> tuple[str | None, str]:
    if mode in ("auto", "sparse"):
        try:
            for r in rows_sparse(name, sparse_base):
                if r["vers"] == version:
                    return r["cksum"], "index.crates.io"
        except Exception:  # noqa: BLE001
            if mode == "sparse":
                raise
    for r in rows_github(name):
        if r["vers"] == version:
            return r["cksum"], "github mirror of crates.io-index"
    return None, "not found"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def load_sums(path: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    if not path.exists():
        return out
    for line in path.read_text().splitlines():
        parts = line.split()
        if len(parts) == 2:
            out[parts[1].lstrip("*")] = parts[0]
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="Verify .crate archives against the crates.io index")
    ap.add_argument("-d", "--dir", type=Path, default=Path("crates"))
    ap.add_argument("--index", choices=["auto", "sparse", "github"], default="auto")
    ap.add_argument("--index-url", default="https://index.crates.io/")
    ap.add_argument("-j", "--jobs", type=int, default=6)
    ap.add_argument("--strict", action="store_true", help="exit non-zero if anything fails")
    ap.add_argument("-q", "--quiet", action="store_true")
    args = ap.parse_args()

    recorded = load_sums(args.dir / "SHA256SUMS")
    files = sorted(args.dir.glob("*.crate"))
    if not files:
        print(f"no .crate files in {args.dir}")
        return 0

    def check(f: Path):
        m = NAME_RE.match(f.stem)
        if not m:
            return (f.name, "unparsable-name", "", "", "")
        name, ver = m["name"], m["ver"]
        local = sha256(f)
        rec = recorded.get(f.name)
        try:
            up, where = cksum_for(name, ver, args.index, args.index_url)
        except Exception as exc:  # noqa: BLE001
            return (f.name, "index-error", local, str(exc), "")
        if rec is not None and rec != local:
            return (f.name, "SHA256SUMS-mismatch", local, rec, where)
        if up is None:
            return (f.name, "not-in-index", local, "", where)
        return (f.name, "verified" if up == local else "MISMATCH", local, up, where)

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        results = list(pool.map(check, files))

    good = [r for r in results if r[1] == "verified"]
    bad = [r for r in results if r[1] != "verified"]
    if not args.quiet:
        for name, status, local, up, where in bad:
            print(f"✗ {name}: {status}  local={local[:16]}… upstream={(up or '-')[:16]}… ({where})")
    print(f"\n{len(good)}/{len(results)} archives verified against the crates.io index "
          f"({sum(f.stat().st_size for f in files)/1048576:.1f} MB)")
    if bad:
        print("failed: " + ", ".join(r[0] for r in bad))
    return 1 if (bad and args.strict) else 0


if __name__ == "__main__":
    sys.exit(main())
