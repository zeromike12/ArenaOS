#!/usr/bin/env bash
# Build the Rust std/sysroot crates for ArenaOS's bare-metal targets, offline.
#
# WHY THIS EXISTS (docs/DEV-ENV.md): the sandbox cannot `rustup target add
# x86_64-unknown-uefi` (no static.rust-lang.org). But rustc knows the targets
# natively — only the precompiled libraries are missing. This script rebuilds
# exactly those (core, compiler_builtins, alloc, panic_abort + the two
# rustc_std_workspace_* shims) from the official rust-lang/rust sources at
# the tag matching the toolchain version, using the same flags the official
# build uses:
#
#   * RUSTC_BOOTSTRAP=1        — permits the library sources' nightly-gated
#                                features at build time (never for kernel code)
#   * -Z force-unstable-if-unmarked — sysroot-crate stability marking
#   * compiler_builtins cfgs are produced by compiling and running ITS OWN
#     build.rs (libm/configure.rs), fed with cfg values queried from
#     `rustc --print cfg` for the target (incl. target_has_reliable_f16/f128)
#
# Usage:
#   ARENA_RUST_PREFIX=/opt/rust/prefix ARENA_RUST_SRC=/opt/build/rust-src \
#       tools/build-sysroot.sh x86_64-unknown-uefi [more targets...]
set -euo pipefail

RUST="${ARENA_RUST_PREFIX:-/opt/rust/prefix}"
SRC="${ARENA_RUST_SRC:-/opt/build/rust-src}/library"
export RUSTC_BOOTSTRAP=1
export PATH="$RUST/bin:$PATH"
export LD_LIBRARY_PATH="$RUST/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

if [[ ! -d "$SRC/core" ]]; then
    echo "error: rust sources not found at $SRC" >&2
    echo "       run tools/dev-env/bootstrap.sh first" >&2
    exit 1
fi

# Guard: source tag must match the toolchain version (mixed versions produce
# a sysroot that rustc silently rejects or mis-links).
RUSTC_VER="$("$RUST/bin/rustc" --version | awk '{print $2}')"
SRC_VER="$(cd "$SRC/.." && git describe --tags --exact-match 2>/dev/null | tr -d '^' || echo unknown)"
if [[ "$SRC_VER" != "unknown" && "$SRC_VER" != "$RUSTC_VER" ]]; then
    echo "error: rust source tag '$SRC_VER' != toolchain '$RUSTC_VER'" >&2
    exit 1
fi

build_target() {
    local TARGET="$1"
    local OUT="$RUST/lib/rustlib/$TARGET/lib"
    mkdir -p "$OUT"
    local FLAGS="--target $TARGET --edition 2024 --crate-type rlib -C opt-level=2 \
        -C debuginfo=0 -C embed-bitcode=no -Z force-unstable-if-unmarked \
        -C panic=abort --out-dir $OUT"

    echo "== [$TARGET] core"
    # shellcheck disable=SC2086
    rustc $FLAGS --crate-name core "$SRC/core/src/lib.rs"

    echo "== [$TARGET] compiler_builtins (build.rs-driven cfgs)"
    local CB="$SRC/compiler-builtins/compiler-builtins"
    local TMP; TMP=$(mktemp -d)
    rustc --edition 2024 -o "$TMP/cb-build" "$CB/build.rs"

    local TCFG; TCFG=$(rustc --target "$TARGET" --print cfg)
    getcfg() { echo "$TCFG" | sed -n "s/^$1=\"\\(.*\\)\"$/\\1/p"; }
    hascfg() { echo "$TCFG" | grep -qx "$1" && echo 1 || true; }
    local TARGET_FEATURE; TARGET_FEATURE=$(echo "$TCFG" | sed -n 's/^target_feature="\(.*\)"$/\1/p' | paste -sd,)
    local TARGET_FAMILY; TARGET_FAMILY=$(echo "$TCFG" | sed -n 's/^target_family="\(.*\)"$/\1/p' | paste -sd,)

    ( cd "$CB" && TARGET="$TARGET" CARGO_MANIFEST_DIR="$CB" OUT_DIR="$TMP" OPT_LEVEL=2 \
      CARGO_CFG_TARGET_ARCH="$(getcfg target_arch)" \
      CARGO_CFG_TARGET_OS="$(getcfg target_os)" \
      CARGO_CFG_TARGET_ENV="$(getcfg target_env)" \
      CARGO_CFG_TARGET_VENDOR="$(getcfg target_vendor)" \
      CARGO_CFG_TARGET_FAMILY="$TARGET_FAMILY" \
      CARGO_CFG_TARGET_FEATURE="$TARGET_FEATURE" \
      CARGO_CFG_TARGET_HAS_RELIABLE_F16="$(hascfg target_has_reliable_f16)" \
      CARGO_CFG_TARGET_HAS_RELIABLE_F128="$(hascfg target_has_reliable_f128)" \
      CARGO_FEATURE_COMPILER_BUILTINS=1 CARGO_FEATURE_ARCH=1 CARGO_FEATURE_MEM=1 \
      "$TMP/cb-build" > "$TMP/cb-out.txt" )

    local CB_CFGS
    CB_CFGS=$(grep -oE 'cargo:{1,2}rustc-cfg=.*' "$TMP/cb-out.txt" \
              | sed -E 's/cargo:{1,2}rustc-cfg=//' | sed 's/^/--cfg /' | tr '\n' ' ')
    echo "   cfgs: $CB_CFGS"
    # shellcheck disable=SC2086
    rustc $FLAGS --crate-name compiler_builtins --extern core="$OUT/libcore.rlib" \
        --cfg 'feature="compiler-builtins"' --cfg 'feature="unmangled-names"' \
        --cfg 'feature="arch"' --cfg 'feature="mem"' $CB_CFGS \
        "$CB/src/lib.rs"

    echo "== [$TARGET] rustc_std_workspace_core"
    # shellcheck disable=SC2086
    rustc $FLAGS --crate-name rustc_std_workspace_core \
        --extern core="$OUT/libcore.rlib" \
        --extern compiler_builtins="$OUT/libcompiler_builtins.rlib" \
        "$SRC/rustc-std-workspace-core/lib.rs"

    echo "== [$TARGET] alloc"
    # shellcheck disable=SC2086
    rustc $FLAGS --crate-name alloc --extern core="$OUT/libcore.rlib" \
        --extern compiler_builtins="$OUT/libcompiler_builtins.rlib" \
        "$SRC/alloc/src/lib.rs"

    echo "== [$TARGET] rustc_std_workspace_alloc"
    # shellcheck disable=SC2086
    rustc $FLAGS --crate-name rustc_std_workspace_alloc \
        --extern alloc="$OUT/liballoc.rlib" \
        "$SRC/rustc-std-workspace-alloc/lib.rs"

    echo "== [$TARGET] panic_abort"
    # shellcheck disable=SC2086
    rustc $FLAGS --crate-name panic_abort \
        --extern core="$OUT/librustc_std_workspace_core.rlib" \
        "$SRC/panic_abort/src/lib.rs"

    echo "== [$TARGET] DONE: $(ls "$OUT" | tr '\n' ' ')"
    rm -rf "$TMP"
}

if [[ $# -eq 0 ]]; then
    set -- x86_64-unknown-uefi x86_64-unknown-none
fi
for t in "$@"; do
    build_target "$t"
done
