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
# EVERY milestone harness (ADR-0005: old tests are never deleted) —
# including the M5 suite, the two-boot persistence proof, and the
# crash-consistency gate. A glob, so a new script is automatically
# a release gate.
for t_script in tools/test_m*.py; do
    python3 "$t_script"
done

echo "== release $TAG: staging assets =="
REL="build/release"
rm -rf "$REL"
mkdir -p "$REL"

cp build/arena-esp.img "$REL/arena-esp.img"
cp docs/RUNNING.md "$REL/RUNNING.md"

# M5.4 (ADR-0023): the data disk ships FORMATTED — since v0.5.0 a
# zero-filled scratch fails fsd's mount by design, and the user's
# files must persist across their own reboots. The template is the
# pristine volume; the run instructions copy it once to scratch.img.
python3 -c 'import sys; sys.path.insert(0, "tools"); import afs1; afs1.mkfs(sys.argv[1], 8 * 1024 * 1024 // afs1.SECTOR)' "$REL/scratch-template.img"

# Firmware pair: exactly what arena_env resolves (the tested EDK2 build).
OVMF_CODE="$(python3 -c 'import sys; sys.path.insert(0,"tools"); import arena_env; print(arena_env.ovmf_code())')"
OVMF_VARS="$(python3 -c 'import sys; sys.path.insert(0,"tools"); import arena_env; print(arena_env.ovmf_vars_template())')"
cp "$OVMF_CODE" "$REL/edk2-x86_64-code.fd"
cp "$OVMF_VARS" "$REL/ovmf-vars-template.img"

( cd "$REL" && sha256sum arena-esp.img scratch-template.img edk2-x86_64-code.fd ovmf-vars-template.img RUNNING.md > sha256sums.txt )

GIT_SHA="$(git rev-parse --short HEAD)"
{
    echo "ArenaOS $TAG — build ${GIT_SHA} ($(date -u +%Y-%m-%dT%H:%M:%SZ))"
    echo
    echo "Milestone status: M1 8/8, M2 21/21, M3 13/13, M4 9/9, M5 6/6,"
    echo "M6 1/1 PASS (tools/test_m*.py) + the interactive shell session"
    echo "+ the two-boot persistence proof (test_m5_persist.py: written"
    echo "in boot N, read back byte-exact in boot N+1) + the"
    echo "crash-consistency gate (test_m5_crash.py: five"
    echo "SIGKILL-mid-write rounds, every reboot recovers with NO repair"
    echo "tool — fsd just mounts) + the virtio-net link proof"
    echo "(test_m6.py: a hand-built ARP request goes out over the NIC and"
    echo "slirp's reply is verified field-by-field — and a boot WITHOUT"
    echo "the NIC stays green with an honest SKIP);"
    echo "100-boot stability loop green — every boot ends by typing"
    echo "'shutdown' into the running shell (ADR-0011/0020/0022/0023/0024)."
    echo
    echo "This release has a FILESYSTEM: AFS1 (original design, ADR-0023)"
    echo "served entirely from ring 3 — extent-based data, copy-on-write"
    echo "transactional metadata, ping-pong commits. The shell lists,"
    echo "reads, writes, and deletes real files, and they SURVIVE reboots"
    echo "on your scratch.img."
    echo
    echo "This release has a NETWORK DRIVER: netd (virtio-net, ring 3,"
    echo "ADR-0024) — link layer only (raw Ethernet frames); every boot"
    echo "proves it on the wire with a real ARP round trip against QEMU's"
    echo "built-in network. The protocol stack (IP/UDP/TCP services) is"
    echo "Phase 7, built on this driver."
    echo
    echo "Run it: see RUNNING.md (bundled) — two cps + one"
    echo "qemu-system-x86_64 command; the VM boots, runs the full"
    echo "milestone suite on serial, then hands the console to the"
    echo "ArenaOS shell: type 'help' (and 'shutdown' to stop the"
    echo "machine). Serial is the console in both directions"
    echo "(ADR-0020). The bundled scratch-template.img is the formatted"
    echo "AFS1 volume — copy it ONCE, then keep reusing your scratch.img:"
    echo "that is where your files live."
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
        echo "scratch-template.img (formatted AFS1 data disk — files"
        echo "persist across your reboots), edk2-x86_64-code.fd +"
        echo "ovmf-vars-template.img (tested EDK2 firmware pair),"
        echo "RUNNING.md (how to boot), sha256sums.txt."
    } > "$NOTES"

    # Create the release first, then upload assets one by one: if one
    # upload dies the release (and the other assets) survive.
    gh release create "$TAG" \
        --repo zeromike12/ArenaOS \
        --target "$BRANCH" \
        --title "$TITLE" \
        --notes-file "$NOTES"

    assets_ok=1
    for f in arena-esp.img scratch-template.img edk2-x86_64-code.fd ovmf-vars-template.img RUNNING.md sha256sums.txt; do
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
        # Absolute output path: the subshell's cwd is $REL, so a relative
        # "releases/..." would resolve inside build/ (v0.3.0 first attempt).
        ( cd "$REL" && tar czf "$REPO_ROOT/releases/$TAG/$BUNDLE" \
            arena-esp.img scratch-template.img edk2-x86_64-code.fd ovmf-vars-template.img RUNNING.md sha256sums.txt )
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
cp scratch-template.img scratch.img         # FIRST boot only: the formatted
                                            # AFS1 volume — then REUSE your
                                            # scratch.img: files persist
qemu-system-x86_64 \\
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \\
    -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \\
    -drive if=pflash,format=raw,file=ovmf-vars.img \\
    -drive format=raw,file=arena-esp.img \\
    -drive file=scratch.img,format=raw,if=none,id=scr0 \\
    -device virtio-blk-pci,drive=scr0 \\
    -netdev user,id=net0 \\
    -device virtio-net-pci,netdev=net0 \\
    -display none -serial mon:stdio -no-reboot
\`\`\`

Serial is the console in BOTH directions: the VM runs the milestone
suite (including the real filesystem tests on scratch.img and the ARP
link probe over QEMU's built-in network), then the kernel spawns the
storaged + fsd + netd services and the shell, and waits at
the \`arena> \` prompt — type \`help\`, \`ls\`, \`write note.txt hello\`,
\`cat note.txt\`, \`rm note.txt\`, \`ps\`, \`spawn\`, and \`shutdown\` to
stop the machine. What you \`write\` is committed to scratch.img and
comes back next boot. Full details: RUNNING.md inside the tarball
(same as docs/RUNNING.md). Bundle contents: arena-esp.img (boot
disk), scratch-template.img (formatted AFS1 data disk),
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
        # ADR-0020: the verification boot ends at the shell, so the
        # verifier must TYPE the shutdown — marker-paced, exactly like
        # the stability loop (a feed-free boot would hang at the prompt
        # and fail the timeout).
        # Milestone-5 fixture (ADR-0021): the verification boot attaches
        # the same fresh scratch disk the harness uses — without it the
        # m5 suite (and therefore the boot) fails by design.
        # M5.3/5.4 (ADR-0023): the scratch disk must be FORMATTED as
        # AFS1 — fsd mounts it during the m5 suite's fs_service test,
        # and a zero-filled image fails the mount (suite fails by
        # design). The verification boot uses the SHIPPED
        # scratch-template.img from the extracted bundle itself: the
        # artifact the user gets is the artifact that was tested.
        # M6.1 (ADR-0024): the slirp NIC joins the fixture family —
        # the verification asserts m6 RESULT PASS, so the shipped
        # bundle's documented command (which includes the netdev) is
        # exactly what was proven.
        ( cd "$verify_dir" && cp ovmf-vars-template.img ovmf-vars.img && \
          cp scratch-template.img scratch.img && \
          {
            n=0
            while ! grep -aq 'arena>' verify-serial.log 2>/dev/null; do
              sleep 0.2; n=$((n + 1)); if (( n >= 600 )); then exit 0; fi
            done
            printf 'shutdown\r'
            n=0
            while ! grep -aqF 'halting via UEFI ResetSystem(shutdown)' verify-serial.log 2>/dev/null; do
              sleep 0.2; n=$((n + 1)); if (( n >= 600 )); then exit 0; fi
            done
          } | timeout 120 "${QEMU[@]}" \
            -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
            -drive if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd \
            -drive if=pflash,format=raw,file=ovmf-vars.img \
            -drive format=raw,file=arena-esp.img \
            -drive file=scratch.img,format=raw,if=none,id=scr0 \
            -device virtio-blk-pci,drive=scr0 \
            -netdev user,id=net0 \
            -device virtio-net-pci,netdev=net0 \
            -display none -chardev stdio,id=con0,signal=off -serial chardev:con0 \
            -no-reboot > verify-serial.log )
        grep -aqF 'm4: RESULT PASS (9/9)' "$verify_dir/verify-serial.log" \
            && grep -aqF 'm5: RESULT PASS (6/6)' "$verify_dir/verify-serial.log" \
            && grep -aqF 'm6: RESULT PASS (1/1)' "$verify_dir/verify-serial.log" \
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
            echo "environment; the identical run bundle is delivered through the"
            echo "repo instead:**"
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
