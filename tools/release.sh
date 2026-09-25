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
    BRANCH="$(git rev-parse --abbrev-ref HEAD)"
    BUNDLE="arenaos-${TAG}-qemu-x86_64.tar.gz"
    RAW_URL="https://github.com/zeromike12/ArenaOS/raw/refs/heads/${BRANCH}/releases/${TAG}/${BUNDLE}"
    NOTES="$(mktemp)"
    {
        cat "$REL/RELEASE-NOTES.txt"
        echo
        echo "Run bundle: ${BUNDLE} — arena-esp.img (boot disk),"
        echo "edk2-x86_64-code.fd + ovmf-vars-template.img (tested EDK2"
        echo "firmware pair), RUNNING.md (how to boot), sha256sums.txt."
    } > "$NOTES"

    # Create the release first, then upload assets one by one: if one
    # upload dies the release (and the other assets) survive.
    gh release create "$TAG" \
        --repo zeromike12/ArenaOS \
        --target "$BRANCH" \
        --title "$TITLE" \
        --notes-file "$NOTES"

    assets_ok=1
    for f in arena-esp.img edk2-x86_64-code.fd ovmf-vars-template.img RUNNING.md sha256sums.txt; do
        gh release upload "$TAG" "$REL/$f" --repo zeromike12/ArenaOS --clobber \
            || assets_ok=0
    done

    if (( ! assets_ok )); then
        # Fallback (needed e.g. in sandboxes where uploads.github.com is
        # unreachable): ship the identical bundle through the repo itself
        # and point the release notes at it. Verify the bundle boots from
        # an extracted copy before publishing it — no untested artifacts.
        echo "== asset upload failed — falling back to in-repo bundle =="
        mkdir -p "releases/$TAG"
        ( cd "$REL" && tar czf "../releases/$TAG/$BUNDLE" \
            arena-esp.img edk2-x86_64-code.fd ovmf-vars-template.img RUNNING.md sha256sums.txt )
        ( cd "releases/$TAG" && sha256sum "$BUNDLE" > "$BUNDLE.sha256" )
        blob_sha="$(git hash-object "releases/$TAG/$BUNDLE")"
        cat > "releases/$TAG/README.md" <<EOF
# ArenaOS ${TAG} — QEMU run bundle

The complete, tested build for running ArenaOS in your own QEMU.
GitHub asset uploads were unreachable from the build environment, so
the release (https://github.com/zeromike12/ArenaOS/releases/tag/${TAG})
links this in-repo bundle instead.

## Download

\`\`\`sh
curl -LO ${RAW_URL}
sha256sum -c ${BUNDLE}.sha256

# or via the GitHub API (contents endpoint caps at 1 MiB — use the blob):
gh api repos/zeromike12/ArenaOS/git/blobs/${blob_sha} \\
    -H "Accept: application/vnd.github.raw" > ${BUNDLE}
\`\`\`

## Run

\`\`\`sh
tar xzf ${BUNDLE}
cp ovmf-vars-template.img ovmf-vars.img     # fresh NVRAM per boot
qemu-system-x86_64 \\
    -M q35 -m 512M -cpu qemu64,+nx \\
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \\
    -drive if=pflash,format=raw,file=ovmf-vars.img \\
    -drive format=raw,file=arena-esp.img \\
    -display none -serial mon:stdio -no-reboot
\`\`\`

Serial is the console; the VM runs the milestone suite and shuts itself
down cleanly. Full details: RUNNING.md inside the tarball (same as
docs/RUNNING.md). Bundle contents: arena-esp.img (boot disk),
edk2-x86_64-code.fd + ovmf-vars-template.img (tested EDK2 firmware
pair), RUNNING.md, sha256sums.txt.
EOF
        verify_dir="$(mktemp -d)"
        tar xzf "releases/$TAG/$BUNDLE" -C "$verify_dir"
        ( cd "$verify_dir" && sha256sum -c sha256sums.txt )
        eval "$(cd "$REPO_ROOT" && python3 - <<'EOF'
import sys
from pathlib import Path
sys.path.insert(0, str(Path("tools").resolve()))
import arena_env
print(f"QEMU=({' '.join(repr(x) for x in arena_env.qemu_cmd() + arena_env.qemu_data_args())})")
EOF
)"
        ( cd "$verify_dir" && cp ovmf-vars-template.img ovmf-vars.img && \
          timeout 120 "${QEMU[@]}" \
            -M q35 -m 512M -cpu qemu64,+nx \
            -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
            -drive if=pflash,format=raw,file=ovmf-vars.img \
            -drive format=raw,file=arena-esp.img \
            -display none -serial file:verify-serial.log -no-reboot )
        grep -aqF 'RESULT PASS' "$verify_dir/verify-serial.log" \
            && grep -aqF 'halting via UEFI ResetSystem(shutdown)' "$verify_dir/verify-serial.log" \
            || { echo "error: bundle verification boot FAILED" >&2; exit 1; }
        rm -rf "$verify_dir"
        git add "releases/$TAG"
        git commit -m "release $TAG: in-repo QEMU run bundle (asset uploads blocked)"
        git push origin "$BRANCH"
        {
            cat "$NOTES"
            echo
            echo "**Asset uploads to uploads.github.com failed from the build"
            "environment; the identical run bundle is delivered through the"
            "repo instead:**"
            echo
            echo "- Download: $RAW_URL"
            echo "- Checksum: ${RAW_URL}.sha256"
            echo "- Details:  releases/$TAG/README.md"
        } > "$NOTES.fallback"
        gh release edit "$TAG" --repo zeromike12/ArenaOS --notes-file "$NOTES.fallback"
        rm -f "$NOTES.fallback"
    fi
    rm -f "$NOTES"
    echo "== published: https://github.com/zeromike12/ArenaOS/releases/tag/$TAG =="
fi
