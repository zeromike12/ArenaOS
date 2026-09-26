#!/usr/bin/env bash
# Interactive boot: serial console on stdio (Ctrl-A X to exit QEMU).
# Use this to *watch* the system boot; use tools/run_tests.sh for verdicts.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "$REPO_ROOT/tools/dev-env/env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$REPO_ROOT/tools/dev-env/env.sh"
fi

"$REPO_ROOT/tools/build.sh" --image

# Resolve QEMU/OVMF exactly like the test harness does.
eval "$(python3 - <<'EOF'
import sys
from pathlib import Path
sys.path.insert(0, str(Path("tools").resolve()))
import arena_env
print(f"QEMU=({' '.join(repr(x) for x in arena_env.qemu_cmd() + arena_env.qemu_data_args())})")
print(f"OVMF_CODE={arena_env.ovmf_code()}")
print(f"OVMF_VARS={arena_env.ovmf_vars_template()}")
print(f"SCRATCH=({' '.join(repr(x) for x in arena_env.scratch_disk_args())})")
EOF
)"

mkdir -p "$REPO_ROOT/build"
cp "$OVMF_VARS" "$REPO_ROOT/build/ovmf-vars-interactive.img"

exec "${QEMU[@]}" \
    -M q35 -m 512M -cpu qemu64,+nx,+smep,+smap \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
    -drive if=pflash,format=raw,file="$REPO_ROOT/build/ovmf-vars-interactive.img" \
    -drive format=raw,file="$REPO_ROOT/build/arena-esp.img" \
    "${SCRATCH[@]}" \
    -display none -serial mon:stdio -no-reboot
