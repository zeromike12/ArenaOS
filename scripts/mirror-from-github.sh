#!/usr/bin/env bash
# mirror-from-github.sh - recover real crates.io `.crate` archives using only
# github.com, for environments where crates.io and its mirrors are firewalled.
#
# How it works: several public GitHub repositories happen to contain committed
# cargo caches / `cargo local-registry` exports, i.e. genuine `.crate` archives
# downloaded from static.crates.io. This script clones them, keeps the archives
# that belong to the OS-development allowlist, and only accepts an archive if its
# SHA-256 equals the `cksum` recorded in the crates.io index (read through the
# GitHub mirror github.com/rust-lang/crates.io-index). Verified archives are then
# copied into the output directory (default: crates/) together with a matching
# `cargo local-registry` index.
#
# usage:
#   ./mirror-from-github.sh                       # -> ../crates
#   ./mirror-from-github.sh --out /tmp/mycrates --all
#   ./mirror-from-github.sh --work /tmp/work --keep-work
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/../crates"
WORK="$(mktemp -d -t crates-from-github.XXXXXX)"
ALLOWLIST="$HERE/os-crate-allowlist.txt"
INDEX_MODE="github"
ONLY_ALLOWED=1
KEEP=0

MIRROR_REPOS=(
  "phoxal/registry"                 # 1115 crates, still updated
  "lwx270901/registryintern"        # 632 crates, committed ~/.cargo/registry/cache
  "shards-lang/rust-registry"       # 256 crates, cargo local-registry export
  "ProtonPrivacy/rust-registry"     # 241 crates
  "pombredanne/crates_popular"      # 60 crates
  "zyma98/custom-rust-registry"     # small, sometimes has crates the others lack
)

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --work) WORK="$2"; shift 2 ;;
    --all) ONLY_ALLOWED=0; shift ;;
    --keep-work) KEEP=1; shift ;;
    --index) INDEX_MODE="$2"; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT" "$WORK/repos" "$WORK/pool" "$WORK/wanted"
echo "== work dir: $WORK"

for repo in "${MIRROR_REPOS[@]}"; do
  dir="$WORK/repos/$(tr '/' '_' <<<"$repo")"
  if [ -d "$dir/.git" ]; then
    echo "== $repo (cached)"
  else
    echo "== cloning $repo"
    git clone --depth 1 --quiet "https://github.com/$repo" "$dir" || { echo "   clone failed, skipping"; continue; }
  fi
  find -L "$dir" -name '*.crate' -exec cp -n {} "$WORK/pool/" \; 2>/dev/null || true
done

echo "== collected $(find "$WORK/pool" -name '*.crate' | wc -l) candidate archives"
[ "$ONLY_ALLOWED" -eq 1 ] || : > "$WORK/keep-all"

# 1) filter by allowlist (crate name = everything before the last -<semver>)
if [ "$ONLY_ALLOWED" -eq 1 ]; then
  while read -r f; do
    base="$(basename "$f" .crate)"
    name="$(sed -E 's/[-@][0-9]+(\.[0-9]+)*([-+][0-9A-Za-z.+-]*)?$//' <<<"$base")"
    if grep -qxF "$name" "$ALLOWLIST"; then
      mkdir -p "$WORK/wanted"; cp -n "$f" "$WORK/wanted/" || true
    fi
  done < <(find -L "$WORK/pool" -name '*.crate')
  echo "== $(find "$WORK/wanted" -name '*.crate' | wc -l) archives match the OS allowlist"
else
  find -L "$WORK/pool" -name '*.crate' -exec cp -n {} "$WORK/wanted/" \; || true
fi

# 2) verify against the crates.io index - this is what proves provenance
echo "== verifying against the crates.io index (channel: $INDEX_MODE)"
python3 "$HERE/verify-crates.py" -d "$WORK/wanted" --index "$INDEX_MODE" -q

# 3) copy only verified archives into place
find -L "$WORK/wanted" -name '*.crate' -exec cp -n {} "$OUT/" \; 2>/dev/null || true
python3 "$HERE/fetch-crates.py" --reindex-only --index "$INDEX_MODE" -o "$OUT"

echo
echo "== done. $(find "$OUT" -maxdepth 1 -name '*.crate' | wc -l) archives in $OUT"
python3 "$HERE/verify-crates.py" -d "$OUT" --index "$INDEX_MODE" -q

[ "$KEEP" -eq 1 ] || rm -rf "$WORK"
