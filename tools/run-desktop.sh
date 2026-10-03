#!/usr/bin/env bash
# Interactive ArenaOS Phase-10 desktop in QEMU (window or VNC).
#
# Run from the repository root:
#   bash tools/run-desktop.sh            boot the prebuilt image (needs only QEMU + Python 3)
#   bash tools/run-desktop.sh --build    rebuild the image from this checkout first
#   bash tools/run-desktop.sh --fresh    start from a new empty disk (combinable with --build)
#
# Without --fresh the disk in build/interactive/ is reused, so user files and
# the light/dark + motion settings persist between boots.
#
# Environment:
#   ARENA_QEMU      QEMU command (default: qemu-system-x86_64 on PATH, or the
#                   pinned /opt/qemu build from tools/dev-env/bootstrap.sh)
#   DISPLAY_OPT     QEMU -display backend: gtk, sdl, cocoa ... Default: the first
#                   of gtk/cocoa/sdl this QEMU supports, otherwise VNC on
#                   localhost:5901 (connect any VNC viewer).
#   VIDEO           gop (default, 800x600 firmware framebuffer) | gpu (virtio-gpu
#                   800x600) | gpu640 (virtio-gpu 640x480)
#   ARENA_OVMF_CODE / ARENA_OVMF_VARS   override the bundled EDK2 firmware
#
# The serial shell stays in this terminal (type `shutdown` at `arena>` for a
# clean stop; Ctrl-A then X force-quits QEMU). This is a development image of
# the Opus design branch, not a qualified Phase-10 release.
set -euo pipefail

BUILD=0
FRESH=0
for arg in "$@"; do
    case "$arg" in
        --build) BUILD=1 ;;
        --fresh) FRESH=1 ;;
        -h|--help) sed -n '2,26p' "$0"; exit 0 ;;
        *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
    esac
done

ROOT="$(pwd)"
[[ -f tools/arena_env.py ]] || { echo "run this from the ArenaOS repository root" >&2; exit 1; }
PREBUILT="$ROOT/releases/checkpoints/phase10-opus-design/prebuilt"
W="$ROOT/build/interactive"
mkdir -p "$W"

# 1. Boot image: rebuilt from source, or the checksummed prebuilt one.
if [[ $BUILD == 1 ]]; then
    [[ -f tools/dev-env/env.sh ]] && source tools/dev-env/env.sh
    unset ARENA_GRAPHICS_FIXTURE # production desktop profile
    echo "[run-desktop] building from source (log: build/interactive/build.log) ..."
    tools/build.sh --image >"$W/build.log" 2>&1 ||
        { echo "build failed, see $W/build.log" >&2; exit 1; }
    ESP="$ROOT/build/arena-esp.img"
else
    ESP="$W/arena-esp.img"
fi

# 2. Unpack and verify the bundled image/firmware, prepare the disk and
#    fresh firmware variables, and resolve QEMU.
eval "$(python3 - "$PREBUILT" "$W" "$BUILD" "$FRESH" <<'EOF'
import gzip, hashlib, os, shlex, shutil, sys
from pathlib import Path
sys.path.insert(0, 'tools')
import arena_env
prebuilt, w = Path(sys.argv[1]), Path(sys.argv[2])
build, fresh = sys.argv[3] == '1', sys.argv[4] == '1'
sums = {}
for line in (prebuilt / 'SHA256SUMS').read_text().splitlines():
    digest, name = line.split()
    sums[name] = digest
def unpack(name):
    out = w / name
    if not out.exists() or hashlib.sha256(out.read_bytes()).hexdigest() != sums[name]:
        packed = prebuilt / (name + '.gz')
        if hashlib.sha256(packed.read_bytes()).hexdigest() != sums[name + '.gz']:
            sys.exit(f'checksum mismatch: {packed}')
        out.write_bytes(gzip.decompress(packed.read_bytes()))
        if hashlib.sha256(out.read_bytes()).hexdigest() != sums[name]:
            sys.exit(f'checksum mismatch after unpacking: {out}')
    return out
if not build:
    unpack('arena-esp.img')
code = os.environ.get('ARENA_OVMF_CODE') or str(unpack('edk2-x86_64-code.fd'))
template = os.environ.get('ARENA_OVMF_VARS') or str(unpack('edk2-i386-vars.fd'))
shutil.copyfile(template, w / 'vars.img')
disk = w / 'disk.img'
if fresh or not disk.exists():
    shutil.copyfile(arena_env.make_scratch_disk(), disk)
print('QEMU=(' + ' '.join(map(shlex.quote, arena_env.qemu_cmd() + arena_env.qemu_data_args())) + ')')
print('OVMF_CODE=' + shlex.quote(code))
EOF
)"
python3 - "$ESP" <<'EOF'
import hashlib, sys
print(f'[run-desktop] booting {sys.argv[1]} sha256={hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest()}')
EOF

# 3. Host peers for the boot-time network self-tests (10.0.2.2:54321 TCP,
#    :1053 UDP); they only answer the guest's own test traffic.
rm -f "$W/peers.out" "$W/serial.log"
# UNIX socket paths are limited to ~104-108 bytes, so keep QMP's socket in a
# short temporary directory rather than under a possibly deep checkout.
SOCK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arena.XXXXXX")"
QMP_SOCK="$SOCK_DIR/qmp.sock"
(cd tools && exec python3 -u network_fixture.py "$W/tcp.log" "$W/dns.log" >"$W/peers.out") &
peers_pid=$!
typist_pid=""
cleanup() {
    kill "$peers_pid" ${typist_pid:+"$typist_pid"} 2>/dev/null || true
    rm -rf "$SOCK_DIR"
}
trap cleanup EXIT
for _ in $(seq 1 50); do
    grep -q READY "$W/peers.out" 2>/dev/null && break
    kill -0 "$peers_pid" 2>/dev/null || break
    sleep 0.1
done
grep -q READY "$W/peers.out" 2>/dev/null ||
    { echo "host test peers could not bind ports 54321/1053 (another ArenaOS run?)" >&2; exit 1; }

# 4. The historical boot sequence waits for the word "arena" on the virtual
#    keyboard once inputd is ready; type it automatically through QMP.
python3 - "$W" "$QMP_SOCK" <<'EOF' &
import sys, time
from pathlib import Path
sys.path.insert(0, 'tools')
import qmp
w = Path(sys.argv[1]); log = w / 'serial.log'
end = time.monotonic() + 180
while time.monotonic() < end:
    if log.is_file() and b'inputd: virtio-input ready' in log.read_bytes():
        break
    time.sleep(0.1)
else:
    sys.exit('[run-desktop] keyboard never became ready; type "arena" in the QEMU window yourself')
c = qmp.Qmp(sys.argv[2], connect_timeout_s=5)
for ch in 'arena':
    c.key(ch)
c.close()
print('\r\n[run-desktop] typed "arena"; the desktop starts now', flush=True)
EOF
typist_pid=$!

# 5. Display: a window when this QEMU has one, otherwise VNC.
if [[ -z "${DISPLAY_OPT:-}" ]]; then
    backends="$("${QEMU[@]}" -display help 2>/dev/null || true)"
    for b in gtk cocoa sdl; do
        if grep -qx "$b" <<<"$backends"; then DISPLAY_OPT=$b; break; fi
    done
fi
if [[ -n "${DISPLAY_OPT:-}" ]]; then
    DISPLAY_ARGS=(-display "$DISPLAY_OPT")
else
    DISPLAY_ARGS=(-display none -vnc 127.0.0.1:1)
    echo "[run-desktop] this QEMU has no window backend: open a VNC viewer on localhost:5901"
fi
case "${VIDEO:-gop}" in
    gpu)    VIDEO_ARGS=(-vga none -device virtio-gpu-pci,xres=800,yres=600) ;;
    gpu640) VIDEO_ARGS=(-vga none -device virtio-gpu-pci,xres=640,yres=480) ;;
    *)      VIDEO_ARGS=() ;;
esac

# 6. Boot.
"${QEMU[@]}" \
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap -boot order=c \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
    -drive if=pflash,format=raw,file="$W/vars.img" \
    -drive format=raw,file="$ESP" \
    -drive file="$W/disk.img",format=raw,if=none,id=scr0 \
    -device virtio-blk-pci,drive=scr0 \
    -netdev user,id=net0 -device virtio-net-pci,netdev=net0 \
    -device virtio-rng-pci \
    -device virtio-keyboard-pci \
    -device virtio-tablet-pci \
    ${VIDEO_ARGS[@]+"${VIDEO_ARGS[@]}"} \
    -qmp unix:"$QMP_SOCK",server=on,wait=off \
    "${DISPLAY_ARGS[@]}" \
    -chardev stdio,id=con0,signal=off,mux=on,logfile="$W/serial.log" \
    -serial chardev:con0 -mon chardev=con0,mode=readline \
    -no-reboot
