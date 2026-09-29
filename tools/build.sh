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

# M6.4 (ADR-0027): the console channel service — ONE crate, TWO images:
# consoled (spawn-registry image 12, the resident virtio-console
# driver, the fifth on the shared virtio core and the first with a
# queue in each direction) and contest (image 13, the m6 suite's port
# round-trip client). Both embed into the kernel via include_bytes!.
echo "== building userspace consoled (userspace/consoled, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/consoled" && cargo build --release )
for CONSOLED_ELF in \
    "$REPO_ROOT/userspace/consoled/target/x86_64-unknown-none/release/arena-consoled" \
    "$REPO_ROOT/userspace/consoled/target/x86_64-unknown-none/release/contest"; do
    if [[ ! -f "$CONSOLED_ELF" ]]; then
        echo "error: consoled image not produced at $CONSOLED_ELF" >&2
        exit 1
    fi
    echo "consoled image: ${CONSOLED_ELF#"$REPO_ROOT"/} ($(stat -c%s "$CONSOLED_ELF") bytes)"
done

# M6.5 (ADR-0028): the fault-injection pair — faultd (spawn-registry
# image 14, the service written to be killed mid-request) and
# faulttest (image 15, the client that must get a typed answer instead
# of waiting forever).
echo "== building userspace faultd (userspace/faultd, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/faultd" && cargo build --release )
for FAULTD_ELF in \
    "$REPO_ROOT/userspace/faultd/target/x86_64-unknown-none/release/arena-faultd" \
    "$REPO_ROOT/userspace/faultd/target/x86_64-unknown-none/release/faulttest"; do
    if [[ ! -f "$FAULTD_ELF" ]]; then
        echo "error: faultd image not produced at $FAULTD_ELF" >&2
        exit 1
    fi
    echo "faultd image: ${FAULTD_ELF#"$REPO_ROOT"/} ($(stat -c%s "$FAULTD_ELF") bytes)"
done

# M7.0 (ADR-0029): the timer-facility proof (spawn-registry image 16).
echo "== building userspace timertest (userspace/timertest, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/timertest" && cargo build --release )
TIMERTEST_ELF="$REPO_ROOT/userspace/timertest/target/x86_64-unknown-none/release/timertest"
if [[ ! -f "$TIMERTEST_ELF" ]]; then
    echo "error: timertest image not produced at $TIMERTEST_ELF" >&2
    exit 1
fi
echo "timertest image: ${TIMERTEST_ELF#"$REPO_ROOT"/} ($(stat -c%s "$TIMERTEST_ELF") bytes)"

# M7.1 (ADR-0030): the network stack service — netstackd (image 17,
# protocol state kept out of the driver) and arptest (image 18).
echo "== building userspace netstackd (userspace/netstackd, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/netstackd" && cargo build --release )
for NS_ELF in \
    "$REPO_ROOT/userspace/netstackd/target/x86_64-unknown-none/release/arena-netstackd" \
    "$REPO_ROOT/userspace/netstackd/target/x86_64-unknown-none/release/arptest"; do
    if [[ ! -f "$NS_ELF" ]]; then
        echo "error: netstackd image not produced at $NS_ELF" >&2
        exit 1
    fi
    echo "netstackd image: ${NS_ELF#"$REPO_ROOT"/} ($(stat -c%s "$NS_ELF") bytes)"
done

# Phase 8.0 bootstrap substrate: a real ring-3 manager image (19).
# It validates caller-held caps and driver readiness and starts the
# production stack; one orderly restart is tested, but 8.0 remains open.
echo "== building userspace servicemgr (userspace/servicemgr, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/servicemgr" && cargo build --release )
MGR_ELF="$REPO_ROOT/userspace/servicemgr/target/x86_64-unknown-none/release/arena-servicemgr"
if [[ ! -f "$MGR_ELF" ]]; then
    echo "error: servicemgr image not produced at $MGR_ELF" >&2
    exit 1
fi
echo "servicemgr image: ${MGR_ELF#"$REPO_ROOT"/} ($(stat -c%s "$MGR_ELF") bytes)"

echo "== building userspace depcheck (userspace/depcheck, x86_64-unknown-none) =="
( cd "$REPO_ROOT/userspace/depcheck" && cargo build --release )
DEPCHECK_ELF="$REPO_ROOT/userspace/depcheck/target/x86_64-unknown-none/release/arena-depcheck"
test -f "$DEPCHECK_ELF"
echo "depcheck image: ${DEPCHECK_ELF#"$REPO_ROOT"/} ($(stat -c%s "$DEPCHECK_ELF") bytes)"

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
