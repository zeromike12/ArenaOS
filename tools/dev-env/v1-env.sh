#!/usr/bin/env bash
# Environment for V1 qualification runs against the pinned Phase 15 backend.
# Source it:  source tools/dev-env/v1-env.sh
#
# Every value points at an artifact whose identity is recorded in
# docs/v1/V2-STATUS.md §3 and re-verifiable with tools/v1_env_evidence.sh.
# Override any of them before a run to point at a different (recorded) backend.

ARENA_PKG="${ARENA_PKG:-$HOME/phase15-pkg}"

# Pinned QEMU 11.1.2 (patched) ELF launched through the private glibc 2.40
# loader, with explicit -accel tcg -L <pkg>/backend/pc-bios (the wrapper adds
# both). arena_env.qemu_cmd() returns exactly this prefix.
export ARENA_QEMU="${ARENA_QEMU:-$HOME/phase16-backend/qemu-patched}"

# Firmware from the same verified package (the archive's own OVMF images).
export ARENA_OVMF_CODE="${ARENA_OVMF_CODE:-$ARENA_PKG/firmware/OVMF_CODE_4M.fd}"
export ARENA_OVMF_VARS="${ARENA_OVMF_VARS:-$ARENA_PKG/firmware/OVMF_VARS_4M.fd}"

# Private Rust 1.97.0 toolchain (official bare-metal target components).
export ARENA_RUST_BIN="${ARENA_RUST_BIN:-/opt/rust/prefix/bin}"
# The private compiler is dynamically linked against its own rustc_driver.
# Put that library directory first so rustc works in a clean/chrooted runner.
if [[ -d /opt/rust/prefix/lib ]]; then
    case "${LD_LIBRARY_PATH:-}" in
        /opt/rust/prefix/lib|/opt/rust/prefix/lib:*) ;;
        *) export LD_LIBRARY_PATH="/opt/rust/prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ;;
    esac
fi

# Documented Clang route: zig cc -target x86_64-freestanding-none.
export ARENA_ZIG="${ARENA_ZIG:-/opt/zig-clang/ziglang/zig}"

# The Phase 15 qualification package (backend controls + verifier inputs).
export ARENA_PHASE15_PKG="${ARENA_PHASE15_PKG:-$ARENA_PKG}"

case ":$PATH:" in
    *":$ARENA_RUST_BIN:"*) ;;
    *) export PATH="$ARENA_RUST_BIN:$PATH" ;;
esac
