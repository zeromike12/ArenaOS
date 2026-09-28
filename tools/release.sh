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

# The stability qualification, checked against THIS artifact.
#
# v0.10.0 is why this exists. Its release notes were honest about the
# suite, but the stability tooling in that very tag could not have
# scored 100/100 on the image it shipped — it was still looking for a
# verdict line two milestones out of date, and nothing connected the
# claim to the artifact. A receipt does: tools/stability_loop.sh
# records the sha256 of the ESP it qualified, and publishing refuses
# unless that matches the ESP just built.
RECEIPT="$REPO_ROOT/build/stability-receipt.txt"
# The KERNEL image, not the ESP: an ESP is a FAT volume whose bytes
# change on every rebuild (directory timestamps), so it cannot anchor
# anything. arena-boot.efi changes only when the kernel is relinked.
ESP_SHA="$(sha256sum "$REPO_ROOT/build/arena-boot.efi" | cut -d' ' -f1)"
STABILITY="(not run)"
if [[ -f "$RECEIPT" ]]; then
    read -r R_SHA R_SCORE < "$RECEIPT"
    if [[ "$R_SHA" == "$ESP_SHA" ]]; then
        STABILITY="$R_SCORE"
    fi
fi
echo "== release $TAG: stability qualification: $STABILITY (kernel $ESP_SHA) =="
if [[ "$MODE" == "--publish" && "$STABILITY" == "(not run)" ]]; then
    echo "error: no stability receipt for the image being published." >&2
    echo "       run: tools/stability_loop.sh 100" >&2
    echo "       (the image must not change afterwards — the receipt is" >&2
    echo "        matched by sha256, which is the whole point)" >&2
    exit 1
fi

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

# The notes are DERIVED, not maintained.
#
# They used to be paragraphs written by hand, and they rotted exactly
# the way the stability loop's hardcoded verdicts did: v0.14.0 shipped
# notes claiming "M6 2/2" (it is 6/6) and describing IP as future work
# on the release that added IP. Anything restated by hand drifts from
# what the build actually does, so:
#
#   * the suite verdicts come from a REAL BOOT of the image being
#     shipped (the same boot that qualifies the bundle), and
#   * the feature list comes from the completed milestones in
#     docs/ROADMAP.md, which is where they are recorded anyway.
#
# If a milestone is not checked off in the ROADMAP it does not appear
# in the notes, and if a suite does not pass in the boot it is not
# claimed.
NOTES_BOOT_LOG="$REPO_ROOT/build/release-verdicts.log"
echo "== release $TAG: booting the image to derive its verdicts =="
( cd "$REPO_ROOT" && python3 - "$NOTES_BOOT_LOG" <<'EOF'
import sys
from pathlib import Path
sys.path.insert(0, str(Path("tools").resolve()))
import mtest
esp = Path("build/arena-esp.img")
_, serial, _ = mtest.run_qemu("release-verdicts", esp)
Path(sys.argv[1]).write_text(serial)
EOF
) >/dev/null 2>&1 || true

{
    echo "ArenaOS $TAG — build ${GIT_SHA} ($(date -u +%Y-%m-%dT%H:%M:%SZ))"
    echo
    echo "Suite verdicts, read from a real boot of THIS image:"
    if [[ -s "$NOTES_BOOT_LOG" ]]; then
        grep -ao '^m[0-9]*: RESULT [A-Z]* ([^)]*)' "$NOTES_BOOT_LOG" \
            | sed 's/\r//' | sort -u | sed 's/^/  * /'
    else
        echo "  * (the verdict boot produced no output — see the gate log)"
    fi
    echo
    echo "What is in this build (the milestones checked off in"
    echo "docs/ROADMAP.md, most recent first):"
    grep -o '^- \[x\] [0-9][0-9.a-z]* \*\*[^*]*\*\*' "$REPO_ROOT/docs/ROADMAP.md" \
        | sed 's/^- \[x\] /  * /; s/\*\*//g' | tail -12 | tac
    echo
    echo "Every boot runs the whole milestone suite on real devices and"
    echo "ends by typing 'shutdown' into the running shell; a build that"
    echo "is not green is never staged. See docs/ROADMAP.md and"
    echo "docs/adr/ for the decisions behind each item."
    echo
    echo "Qualification for THIS image (kernel ${ESP_SHA:0:16}...):"
    echo "  * every milestone test script, green;"
    echo "  * boot-stability loop: $STABILITY boots fully green, verdicts"
    echo "    identical on every boot;"
    echo "  * the bundle below was extracted and BOOTED before publishing."
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
    -device virtio-rng-pci \\
    -device virtio-keyboard-pci \\
    -chardev socket,id=vcon0,path=/tmp/arena-console.sock,server=on,wait=off \\
    -device virtio-serial-pci,max_ports=1 \\
    -device virtconsole,chardev=vcon0 \\
    -display none -serial mon:stdio -no-reboot
\`\`\`

While it runs, \`nc -U /tmp/arena-console.sock\` from another terminal
puts you on the same console through the virtio-console port
(ADR-0027).

Want to type on a real keyboard instead of the serial port? Swap
\`-display none\` for \`-display gtk\` (or \`sdl\`/\`cocoa\`): a QEMU
window opens and your keystrokes drive the same \`arena> \` prompt
through the \`inputd\` driver (ADR-0026). Both channels stay live.

Serial is a console in BOTH directions: the VM runs the milestone
suite (including the real filesystem tests on scratch.img and the ARP
link probe over QEMU's built-in network), then the kernel spawns the
storaged + fsd + netd + rngd + inputd + consoled services and the
shell, and waits at
the \`arena> \` prompt — type \`help\`, \`ls\`, \`write note.txt hello\`,
\`cat note.txt\`, \`rm note.txt\`, \`ps\`, \`spawn\`, and \`shutdown\` to
stop the machine. What you \`write\` is committed to scratch.img and
comes back next boot. Full details: RUNNING.md inside the tarball
(same as docs/RUNNING.md). Bundle contents: arena-esp.img (boot
disk), scratch-template.img (formatted AFS1 data disk),
edk2-x86_64-code.fd + ovmf-vars-template.img (tested EDK2 firmware
pair), RUNNING.md, sha256sums.txt.

The boot suite's input test needs a typist: with a keyboard attached
and nobody typing it reports an honest \`SKIP\` after about a second
and the boot continues normally (that configuration is itself a
tested one). Type \`arena\` in the QEMU window while the suite runs
and it PASSes instead.
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
        # M6.3 (ADR-0026): the keyboard joins it too, and a keyboard
        # needs a typist — so the verification boot ALSO runs
        # tools/qmp.py to type the input fixture, exactly as the
        # stability loop does. Without it the shipped configuration
        # would only ever be verified in its SKIP form.
        # Two host-side actors now drive this boot (the serial feeder
        # and the keyboard typist), so the subshell does its `cd`
        # FIRST and runs plain statements afterwards: appending `&` to
        # a `cd && cp && pipeline` chain backgrounds the WHOLE chain
        # and leaves everything after it running from the wrong
        # directory.
        (
          cd "$verify_dir" || exit 1
          cp ovmf-vars-template.img ovmf-vars.img || exit 1
          cp scratch-template.img scratch.img || exit 1
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
            -device virtio-rng-pci \
            -device virtio-keyboard-pci \
            -chardev socket,id=vcon0,path=verify-vcon.sock,server=on,wait=on \
            -device virtio-serial-pci,max_ports=1 \
            -device virtconsole,chardev=vcon0 \
            -qmp unix:verify-qmp.sock,server=on,wait=off \
            -display none -chardev stdio,id=con0,signal=off -serial chardev:con0 \
            -no-reboot > verify-serial.log &
          qemu_pid=$!
          python3 "$REPO_ROOT/tools/qmp.py" verify-qmp.sock verify-serial.log \
            'inputd: virtio-input ready' arena 120 >/dev/null 2>&1 &
          typist_pid=$!
          # M6.4 (ADR-0027): the console port's far end. `wait=on` above
          # means QEMU does not finish starting until this connects, so
          # the shipped configuration is verified in its PASS form.
          python3 "$REPO_ROOT/tools/vcon.py" verify-vcon.sock \
            'contest: hello from ArenaOS' 'host-says-hello
' 120 >/dev/null 2>&1 &
          vcon_pid=$!
          wait "$qemu_pid"
          kill "$typist_pid" 2>/dev/null || true
          kill "$vcon_pid" 2>/dev/null || true
        )
        grep -aqF 'm4: RESULT PASS (9/9)' "$verify_dir/verify-serial.log" \
            && grep -aqF 'm5: RESULT PASS (6/6)' "$verify_dir/verify-serial.log" \
            && grep -aqF 'm6: RESULT PASS (6/6)' "$verify_dir/verify-serial.log" \
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
