#!/usr/bin/env bash
# Boot-stability loop (docs/TESTING.md): boot the *prebuilt* ESP image N
# times with fresh NVRAM each run and require the full green verdict on
# every single boot — m2 RESULT PASS (21/21), the canonical clean-halt
# line, no PANIC, QEMU exit 0, under a per-boot timeout.
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
EOF
)"

VARS="$REPO_ROOT/build/ovmf-vars-stability.img"
SERIAL="$REPO_ROOT/build/stability-serial.log"
BOOT_TIMEOUT=60          # healthy TCG boot is <10s; hang = failure
RESULT_LINE='m2: RESULT PASS (21/21)'
RESULT_LINE_M3='m3: RESULT PASS (7/7)'
HALT_LINE='halting via UEFI ResetSystem(shutdown)'

pass=0
fail=0
t_start=$(date +%s)
for i in $(seq 1 "$N"); do
    cp "$OVMF_VARS" "$VARS"              # fresh NVRAM every boot
    rm -f "$SERIAL"
    rc=0
    timeout "$BOOT_TIMEOUT" "${QEMU[@]}" \
        -M q35 -m 512M -cpu qemu64,+nx \
        -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
        -drive if=pflash,format=raw,file="$VARS" \
        -drive format=raw,file="$ESP" \
        -display none -serial "file:$SERIAL" -no-reboot || rc=$?

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
