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

# M4.6 (ADR-0020): the minimal shell is embedded the same way —
# spawn-registry image 1, spawned at boot as the initial service.
echo "== building userspace shell (userspace/shell, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/shell" && cargo build --release )
SHELL_ELF="$REPO_ROOT/userspace/shell/target/x86_64-unknown-none/release/arena-shell"
if [[ ! -f "$SHELL_ELF" ]]; then
    echo "error: shell ELF not produced at $SHELL_ELF" >&2
    exit 1
fi
echo "shell image: ${SHELL_ELF#"$REPO_ROOT"/} ($(stat -c%s "$SHELL_ELF") bytes)"

# M5.2 (ADR-0022): the block service — ONE crate, TWO images: the
# storaged driver (spawn-registry image 2, spawned at boot) and blktest
# (image 3, the m5 suite's service-boundary client). Both embed into the
# kernel via include_bytes!, so both must exist before it compiles.
echo "== building userspace storaged (userspace/storaged, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/storaged" && cargo build --release )
for STORAGED_ELF in \
    "$REPO_ROOT/userspace/storaged/target/x86_64-unknown-none/release/arena-storaged" \
    "$REPO_ROOT/userspace/storaged/target/x86_64-unknown-none/release/blktest"; do
    if [[ ! -f "$STORAGED_ELF" ]]; then
        echo "error: storaged image not produced at $STORAGED_ELF" >&2
        exit 1
    fi
    echo "storaged image: ${STORAGED_ELF#"$REPO_ROOT"/} ($(stat -c%s "$STORAGED_ELF") bytes)"
done

# M5.3 (ADR-0023): the filesystem service — ONE crate, TWO images: fsd
# (spawn-registry image 4, the AFS1 server spawned at boot on top of
# storaged) and fstest (image 5, the m5 suite's filesystem client).
# Both embed into the kernel via include_bytes!.
echo "== building userspace fsd (userspace/fsd, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/fsd" && cargo build --release )
for FSD_ELF in \
    "$REPO_ROOT/userspace/fsd/target/x86_64-unknown-none/release/fsd" \
    "$REPO_ROOT/userspace/fsd/target/x86_64-unknown-none/release/fstest"; do
    if [[ ! -f "$FSD_ELF" ]]; then
        echo "error: fsd image not produced at $FSD_ELF" >&2
        exit 1
    fi
    echo "fsd image: ${FSD_ELF#"$REPO_ROOT"/} ($(stat -c%s "$FSD_ELF") bytes)"
done

# M6.1 (ADR-0024): the network service — ONE crate, TWO images: netd
# (spawn-registry image 6, the resident virtio-net driver, link layer
# only) and nettest (image 7, the m6 suite's ARP link-probe client).
# Both embed into the kernel via include_bytes!.
echo "== building userspace netd (userspace/netd, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/netd" && cargo build --release )
for NETD_ELF in \
    "$REPO_ROOT/userspace/netd/target/x86_64-unknown-none/release/arena-netd" \
    "$REPO_ROOT/userspace/netd/target/x86_64-unknown-none/release/nettest"; do
    if [[ ! -f "$NETD_ELF" ]]; then
        echo "error: netd image not produced at $NETD_ELF" >&2
        exit 1
    fi
    echo "netd image: ${NETD_ELF#"$REPO_ROOT"/} ($(stat -c%s "$NETD_ELF") bytes)"
done

# M6.2 (ADR-0025): the entropy service — ONE crate, TWO images: rngd
# (spawn-registry image 8, the resident virtio-rng driver on the shared
# virtio core) and rngtest (image 9, the m6 suite's variance probe).
# Both embed into the kernel via include_bytes!.
echo "== building userspace rngd (userspace/rngd, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/rngd" && cargo build --release )
for RNGD_ELF in \
    "$REPO_ROOT/userspace/rngd/target/x86_64-unknown-none/release/arena-rngd" \
    "$REPO_ROOT/userspace/rngd/target/x86_64-unknown-none/release/rngtest"; do
    if [[ ! -f "$RNGD_ELF" ]]; then
        echo "error: rngd image not produced at $RNGD_ELF" >&2
        exit 1
    fi
    echo "rngd image: ${RNGD_ELF#"$REPO_ROOT"/} ($(stat -c%s "$RNGD_ELF") bytes)"
done

# M6.3 (ADR-0026): the input service — ONE crate, TWO images: inputd
# (spawn-registry image 10, the resident virtio-input keyboard driver,
# the fourth on the shared virtio core) and inputtest (image 11, the m6
# suite's decoded-keystroke client). Both embed into the kernel via
# include_bytes!.
echo "== building userspace inputd (userspace/inputd, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/inputd" && cargo build --release )
for INPUTD_ELF in \
    "$REPO_ROOT/userspace/inputd/target/x86_64-unknown-none/release/arena-inputd" \
    "$REPO_ROOT/userspace/inputd/target/x86_64-unknown-none/release/inputtest"; do
    if [[ ! -f "$INPUTD_ELF" ]]; then
        echo "error: inputd image not produced at $INPUTD_ELF" >&2
        exit 1
    fi
    echo "inputd image: ${INPUTD_ELF#"$REPO_ROOT"/} ($(stat -c%s "$INPUTD_ELF") bytes)"
done

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
