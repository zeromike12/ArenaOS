#!/usr/bin/env bash
# Source this file to put the ArenaOS dev toolchain on PATH:
#   source tools/dev-env/env.sh
# Safe to source repeatedly; no-ops on workstations with system toolchains.

if [[ -x /opt/rust/prefix/bin/cargo ]]; then
    case ":$PATH:" in
        *":/opt/rust/prefix/bin:"*) ;;
        *) export PATH="/opt/rust/prefix/bin:$PATH" ;;
    esac
fi

# LLVM tools shipped with the Rust toolchain (objdump/readobj for PE images).
_ARENA_LLVM_TOOLS="/opt/rust/prefix/lib/rustlib/x86_64-unknown-linux-gnu/bin"
if [[ -d "$_ARENA_LLVM_TOOLS" ]]; then
    export ARENA_LLVM_TOOLS="$_ARENA_LLVM_TOOLS"
fi
