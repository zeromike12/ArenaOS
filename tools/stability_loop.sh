#!/usr/bin/env bash
# Boot-stability loop (docs/TESTING.md): boot the *prebuilt* ESP image N
# times with fresh NVRAM each run and require the full green verdict on
# every single boot — m2 RESULT PASS (21/21), m3 RESULT PASS (13/13), m4
# RESULT PASS (9/9), m5 RESULT PASS (6/6), m6 RESULT PASS (3/3), the
# canonical clean-halt line, no PANIC, QEMU exit 0, under a per-boot
# timeout. Every fixture rides along (the AFS1 scratch disk, the slirp
# NIC, the entropy source, and — since M6.3 — the virtio keyboard) —
# stability means the SHIPPING configuration. Boots therefore use BOTH
# input channels: `shutdown` typed on the serial chardev, and the m6
# input fixture typed on the KEYBOARD through QMP (tools/qmp.py), each
# paced by its own serial marker.
#
# A single green boot proves correctness; a hundred prove the kernel is
# not winning a race (fresh-vars nondeterminism, TCG timing jitter).
#
# Usage: tools/stability_loop.sh [N]        (default N=100)
#        tools/build.sh --image             (must run first)
set -euo pipefail

N="${1:-100}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "$REPO_ROOT/tools/dev-env/env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$REPO_ROOT/tools/dev-env/env.sh"
fi

ESP="$REPO_ROOT/build/arena-esp.img"
if [[ ! -f "$ESP" ]]; then
    echo "error: $ESP missing — run tools/build.sh --image first" >&2
    exit 1
fi

# Resolve QEMU/OVMF exactly like the test harness does.
eval "$(cd "$REPO_ROOT" && python3 - <<'EOF'
import sys
from pathlib import Path
sys.path.insert(0, str(Path("tools").resolve()))
import arena_env
print(f"QEMU=({' '.join(repr(x) for x in arena_env.qemu_cmd() + arena_env.qemu_data_args())})")
print(f"OVMF_CODE={arena_env.ovmf_code()}")
print(f"OVMF_VARS={arena_env.ovmf_vars_template()}")
print(f"SCRATCH=({' '.join(repr(x) for x in arena_env.scratch_disk_args())})")
print(f"NET=({' '.join(repr(x) for x in arena_env.net_args())})")
print(f"RNG=({' '.join(repr(x) for x in arena_env.rng_args())})")
print(f"KBD=({' '.join(repr(x) for x in arena_env.input_args())})")
EOF
)"

VARS="$REPO_ROOT/build/ovmf-vars-stability.img"
SERIAL="$REPO_ROOT/build/stability-serial.log"
QMP_SOCK="$REPO_ROOT/build/qmp-stability.sock"
BOOT_TIMEOUT=60          # healthy TCG boot is <10s; hang = failure
RESULT_LINE='m2: RESULT PASS (21/21)'
RESULT_LINE_M3='m3: RESULT PASS (13/13)'
RESULT_LINE_M4='m4: RESULT PASS (9/9)'
RESULT_LINE_M5='m5: RESULT PASS (6/6)'
RESULT_LINE_M6='m6: RESULT PASS (3/3)'
# The keystrokes the m6 input_service test waits for, typed on the
# virtual keyboard once inputd announces DRIVER_OK.
KEY_MARKER='inputd: virtio-input ready'
KEY_TEXT='arena'
HALT_LINE='halting via UEFI ResetSystem(shutdown)'

pass=0
fail=0
t_start=$(date +%s)
# 0.2 s polls per second of boot timeout: the feeder must never outlive
# QEMU's timeout, or a boot that dies before the prompt wedges the
# pipeline forever (timeout kills QEMU, not the grep loop).
FEED_ITERS=$((BOOT_TIMEOUT * 5))

for i in $(seq 1 "$N"); do
    cp "$OVMF_VARS" "$VARS"              # fresh NVRAM every boot
    # Fresh AFS1 scratch disk EVERY boot (the fixture contract, and
    # load-bearing since M5.3: fs_service's fstest CREATEs a fixed
    # filename — a disk inherited from the previous boot answers
    # FS_ERR_EXISTS and fails the suite by design).
    ( cd "$REPO_ROOT" && python3 -c 'import sys; sys.path.insert(0, "tools"); import arena_env; arena_env.make_scratch_disk()' >/dev/null )
    rm -f "$SERIAL" "$QMP_SOCK"
    rc=0
    # The keyboard typist: waits for inputd's ready marker on the
    # serial log, then types the input fixture through QMP. Bounded by
    # the boot timeout so a dead boot can never leave it behind.
    ( python3 "$REPO_ROOT/tools/qmp.py" "$QMP_SOCK" "$SERIAL" \
        "$KEY_MARKER" "$KEY_TEXT" "$BOOT_TIMEOUT" >/dev/null 2>&1 & )
    # ADR-0020: a healthy boot no longer halts by itself — it ends at
    # the shell. The feeder subshell types 'shutdown' when the shell's
    # prompt appears (marker-paced, never sleep-based), then holds
    # stdin open until the clean-halt declaration lands. A dead boot
    # (panic/hang before the prompt) leaves the feeder spinning until
    # the timeout kills the pipeline — the failure verdict is unchanged.
    {
        n=0
        while ! grep -aq 'arena>' "$SERIAL" 2>/dev/null; do
            sleep 0.2; n=$((n + 1)); if (( n >= FEED_ITERS )); then exit 0; fi
        done
        printf 'shutdown\r'
        n=0
        while ! grep -aqF "$HALT_LINE" "$SERIAL" 2>/dev/null; do
            sleep 0.2; n=$((n + 1)); if (( n >= FEED_ITERS )); then exit 0; fi
        done
    } | timeout "$BOOT_TIMEOUT" "${QEMU[@]}" \
        -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
        -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
        -drive if=pflash,format=raw,file="$VARS" \
        -drive format=raw,file="$ESP" \
        "${SCRATCH[@]}" \
        "${NET[@]}" \
        "${RNG[@]}" \
        "${KBD[@]}" \
        -qmp unix:"$QMP_SOCK",server=on,wait=off \
        -display none -chardev stdio,id=con0,signal=off -serial chardev:con0 \
        -no-reboot > "$SERIAL" 2>/dev/null || rc=$?

    why=""
    if (( rc != 0 )); then
        why="qemu exit rc=$rc (timeout is 124)"
    elif [[ ! -f "$SERIAL" ]]; then
        why="no serial output"
    elif grep -aq 'PANIC' "$SERIAL"; then
        why="kernel PANIC on serial"
    elif ! grep -aqF "$RESULT_LINE" "$SERIAL"; then
        why="missing '$RESULT_LINE'"
    elif ! grep -aqF "$RESULT_LINE_M3" "$SERIAL"; then
        why="missing '$RESULT_LINE_M3'"
    elif ! grep -aqF "$RESULT_LINE_M4" "$SERIAL"; then
        why="missing '$RESULT_LINE_M4'"
    elif ! grep -aqF "$RESULT_LINE_M5" "$SERIAL"; then
        why="missing '$RESULT_LINE_M5'"
    elif ! grep -aqF "$RESULT_LINE_M6" "$SERIAL"; then
        why="missing '$RESULT_LINE_M6'"
    elif ! grep -aqF "$HALT_LINE" "$SERIAL"; then
        why="missing clean-halt declaration"
    fi

    if [[ -z "$why" ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        saved="$REPO_ROOT/build/stability-fail-$i.log"
        cp "$SERIAL" "$saved" 2>/dev/null || true
        echo "boot $i: FAIL — $why (serial saved to ${saved#"$REPO_ROOT"/})"
    fi

    if (( i % 10 == 0 )); then
        echo "progress: $i/$N boots (pass=$pass fail=$fail, $(( $(date +%s) - t_start ))s)"
    fi
done

dt=$(( $(date +%s) - t_start ))
echo "=============================================="
echo "stability: $pass/$N boots fully green in ${dt}s (fail=$fail)"
if (( fail > 0 )); then
    echo "STABILITY: FAIL"
    exit 1
fi
echo "STABILITY: PASS"
