#!/usr/bin/env bash
# Negative-control wrapper: report the real pinned QEMU identity/capability
# list, but request unavailable KVM only for the runner's QMP initialization
# smoke. It never starts an ArenaOS guest.
set -euo pipefail
: "${PHASE15_NEGATIVE_CONTROL_QEMU:?set to the real pinned QEMU path}"
if [[ " $* " == *" -qmp stdio "* ]]; then
    args=("$@")
    for ((i=0; i+1<${#args[@]}; i++)); do
        if [[ "${args[i]}" == "-accel" && "${args[i+1]}" == "tcg" ]]; then
            args[i+1]=kvm
            break
        fi
    done
    exec "$PHASE15_NEGATIVE_CONTROL_QEMU" "${args[@]}"
fi
exec "$PHASE15_NEGATIVE_CONTROL_QEMU" "$@"
