#!/usr/bin/env bash
# Build the ArenaOS kernel image (and optionally the bootable ESP image).
#
#   tools/build.sh            -> build/arena-boot.efi
#   tools/build.sh --image    -> + build/arena-esp.img
#
# Requires the dev environment from tools/dev-env/bootstrap.sh (or a normal
# workstation with cargo + the x86_64-unknown-uefi target).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${ARENA_BUILD_PROFILE:-release}"

if [[ -f "$REPO_ROOT/tools/dev-env/env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$REPO_ROOT/tools/dev-env/env.sh"
fi

PROFILE_FLAG="--release"
PROFILE_DIR="release"
if [[ "$PROFILE" == "debug" ]]; then
    PROFILE_FLAG=""
    PROFILE_DIR="debug"
fi

cd "$REPO_ROOT/kernel"
# shellcheck disable=SC2086
cargo build $PROFILE_FLAG

EFI_SRC="$REPO_ROOT/kernel/target/x86_64-unknown-uefi/$PROFILE_DIR/arena-boot.efi"
mkdir -p "$REPO_ROOT/build"
cp "$EFI_SRC" "$REPO_ROOT/build/arena-boot.efi"
echo "kernel image: build/arena-boot.efi ($(stat -c%s "$REPO_ROOT/build/arena-boot.efi") bytes)"

if [[ "${1:-}" == "--image" ]]; then
    python3 "$REPO_ROOT/tools/espimg.py" "$REPO_ROOT/build/arena-boot.efi" "$REPO_ROOT/build/arena-esp.img"
fi
