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

# M4.1 (ADR-0016): the userspace test payload is embedded into the
# kernel via include_bytes! — it must exist before the kernel compiles.
echo "== building userspace payload (userspace/payload, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/payload" && cargo build --release )
PAYLOAD_ELF="$REPO_ROOT/userspace/payload/target/x86_64-unknown-none/release/arena-payload"
if [[ ! -f "$PAYLOAD_ELF" ]]; then
    echo "error: payload ELF not produced at $PAYLOAD_ELF" >&2
    exit 1
fi
echo "payload image: ${PAYLOAD_ELF#"$REPO_ROOT"/} ($(stat -c%s "$PAYLOAD_ELF") bytes)"

cd "$REPO_ROOT/kernel"
# shellcheck disable=SC2086
cargo build $PROFILE_FLAG

EFI_SRC="$REPO_ROOT/kernel/target/x86_64-unknown-uefi/$PROFILE_DIR/arena-boot.efi"
mkdir -p "$REPO_ROOT/build"
cp "$EFI_SRC" "$REPO_ROOT/build/arena-boot.efi"
echo "kernel image: build/arena-boot.efi ($(stat -c%s "$REPO_ROOT/build/arena-boot.efi") bytes)"

# ADR-0012 invariant: the kernel image contains NO FPU/SSE/MMX
# instructions — the context switch saves callee-saved GPRs + RFLAGS
# only, which is sound exactly while this holds. The audit runs on every
# build (release gate included); if it ever fires, the ADR's escalation
# path applies (-C target-feature=-sse,-sse2,-mmx first, xsave second).
RUSTLIB_BIN="$(dirname "$(dirname "$(command -v rustc)")")/lib/rustlib/x86_64-unknown-linux-gnu/bin"
DISASM=""
if [[ -x "$RUSTLIB_BIN/llvm-objdump" ]]; then
    DISASM="$RUSTLIB_BIN/llvm-objdump"
elif command -v objdump >/dev/null 2>&1; then
    DISASM="$(command -v objdump)"
fi
if [[ -n "$DISASM" ]]; then
    if "$DISASM" -d "$REPO_ROOT/build/arena-boot.efi" | grep -qE '\bx?mm[0-7]\b'; then
        echo "error: FPU/SSE/MMX instruction found in the kernel image —" >&2
        echo "       the ADR-0012 context-switch invariant is violated." >&2
        echo "       See docs/adr/0012-kernel-threads-context-switch.md." >&2
        exit 1
    fi
    echo "ADR-0012 audit: no FPU/SSE/MMX instructions in the image"
else
    echo "warning: no disassembler found — ADR-0012 no-SSE audit SKIPPED" >&2
fi

if [[ "${1:-}" == "--image" ]]; then
    python3 "$REPO_ROOT/tools/espimg.py" "$REPO_ROOT/build/arena-boot.efi" "$REPO_ROOT/build/arena-esp.img"
fi
