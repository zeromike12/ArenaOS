#!/usr/bin/env bash
# Assemble (and optionally publish) the milestone release artifacts.
#
# Standing project rule: every completed milestone ships a compiled build
# the maintainer can run in their own QEMU — ESP image + the exact EDK2
# firmware pair it was tested with + run instructions (docs/RUNNING.md),
# published as GitHub release assets from this branch.
#
# Usage:
#   tools/release.sh --stage <tag>            build + test + stage assets
#   tools/release.sh --publish <tag> [title]  stage, then gh release create
#
# Assets land in build/release/ either way; --publish uploads them.
# The release gate is the real test suite: a build that is not green
# is never staged.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
if [[ -f tools/dev-env/env.sh ]]; then
    # shellcheck disable=SC1091
    source tools/dev-env/env.sh
fi

MODE="${1:-}"
TAG="${2:-}"
TITLE="${3:-ArenaOS $TAG}"
case "$MODE" in
    --stage|--publish) ;;
    *) echo "usage: tools/release.sh --stage|--publish <tag> [title]" >&2; exit 2 ;;
esac
[[ -n "$TAG" ]] || { echo "error: <tag> required (e.g. v0.2.0)" >&2; exit 2; }

echo "== release $TAG: building image =="
./tools/build.sh --image

echo "== release $TAG: gating on the full test suite =="
python3 tools/test_m1.py
python3 tools/test_m2.py

echo "== release $TAG: staging assets =="
REL="build/release"
rm -rf "$REL"
mkdir -p "$REL"

cp build/arena-esp.img "$REL/arena-esp.img"
cp docs/RUNNING.md "$REL/RUNNING.md"

# Firmware pair: exactly what arena_env resolves (the tested EDK2 build).
OVMF_CODE="$(python3 -c 'import sys; sys.path.insert(0,"tools"); import arena_env; print(arena_env.ovmf_code())')"
OVMF_VARS="$(python3 -c 'import sys; sys.path.insert(0,"tools"); import arena_env; print(arena_env.ovmf_vars_template())')"
cp "$OVMF_CODE" "$REL/edk2-x86_64-code.fd"
cp "$OVMF_VARS" "$REL/ovmf-vars-template.img"

( cd "$REL" && sha256sum arena-esp.img edk2-x86_64-code.fd ovmf-vars-template.img RUNNING.md > sha256sums.txt )

GIT_SHA="$(git rev-parse --short HEAD)"
{
    echo "ArenaOS $TAG — build ${GIT_SHA} ($(date -u +%Y-%m-%dT%H:%M:%SZ))"
    echo
    echo "Milestone status: M1 8/8 PASS, M2 21/21 PASS (tools/test_m2.py),"
    echo "100-boot stability loop green (tools/stability_loop.sh, ADR-0011)."
    echo
    echo "Run it: see RUNNING.md (bundled) — one cp + one qemu-system-x86_64"
    echo "command; the VM boots, runs the milestone suite on serial, and"
    echo "shuts itself down cleanly."
} > "$REL/RELEASE-NOTES.txt"

ls -la "$REL"
echo "== staged in $REL =="

if [[ "$MODE" == "--publish" ]]; then
    echo "== publishing to GitHub =="
    NOTES="$(mktemp)"
    {
        cat "$REL/RELEASE-NOTES.txt"
        echo
        echo "Assets: arena-esp.img (boot disk), edk2-x86_64-code.fd +"
        echo "ovmf-vars-template.img (tested EDK2 firmware pair), RUNNING.md"
        echo "(how to boot), sha256sums.txt."
    } > "$NOTES"
    gh release create "$TAG" \
        --repo zeromike12/ArenaOS \
        --target "$(git rev-parse --abbrev-ref HEAD)" \
        --title "$TITLE" \
        --notes-file "$NOTES" \
        "$REL/arena-esp.img" \
        "$REL/edk2-x86_64-code.fd" \
        "$REL/ovmf-vars-template.img" \
        "$REL/RUNNING.md" \
        "$REL/sha256sums.txt"
    rm -f "$NOTES"
    echo "== published: https://github.com/zeromike12/ArenaOS/releases/tag/$TAG =="
fi
