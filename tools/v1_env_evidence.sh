#!/usr/bin/env bash
# V2 environment-recovery receipt collector (docs/v1/V2-STATUS.md §3).
#
# Re-runnable: writes every identity and control output the environment
# milestone claims, so the claims can be re-verified rather than trusted.
# Usage:  bash tools/v1_env_evidence.sh [OUTDIR]
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$REPO/docs/v1/evidence/v2-2026-10-29/env}"
PKG="$HOME/phase15-pkg"
WRAP="$HOME/phase16-backend"
GLIBC="${GLIBC_PREFIX:-$HOME/glibc-2.40}"
ARCHIVE="$REPO/docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz"
mkdir -p "$OUT"

say() { echo "[env-receipt] $*"; }
record() { # record <name> <command...>
    local name="$1"; shift
    { echo "\$ $*"; "$@" 2>&1; echo "[exit=$?]"; } > "$OUT/$name.txt" 2>&1
    say "$name"
}

# ---------------------------------------------------------------- host ----
{
    echo "date_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "host: $(uname -a)"
    echo "nproc: $(nproc)"
    echo "mem_total_kb: $(awk '/MemTotal/{print $2}' /proc/meminfo)"
    echo "glibc: $(ldd --version | head -1)"
    echo "gcc: $(gcc --version | head -1)"
    echo "repo_head: $(git -C "$REPO" rev-parse HEAD)"
    echo "repo_branch: $(git -C "$REPO" rev-parse --abbrev-ref HEAD)"
    echo "repo_dirty:"; git -C "$REPO" status --porcelain | sed 's/^/  /'
} > "$OUT/host.txt" 2>&1
say host

# --------------------------------------------------- tracked QEMU package ----
{
    echo "archive: $ARCHIVE"
    echo "bytes: $(stat -c%s "$ARCHIVE")"
    echo "expected_bytes: 95562916"
    sha256sum "$ARCHIVE"
    echo "--- committed sidecar ---"
    cat "$REPO"/docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz.sha256
    echo "--- verification ---"
    ( cd "$REPO/docs/phase15/artifacts" && sha256sum --check \
        phase15-patched-tcg-qualification.tar.gz.sha256 ) 2>&1
} > "$OUT/qemu-package.txt" 2>&1
say qemu-package

tar -tzf "$ARCHIVE" > "$OUT/qemu-package-listing.txt" 2>&1
say qemu-package-listing

{
    echo "expected_patched_elf: e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff"
    sha256sum "$PKG/backend/qemu-system-x86_64-patched" \
              "$PKG/backend/qemu-system-x86_64-unpatched" 2>&1
    echo "--- runtime libraries ---"
    sha256sum "$PKG"/backend/runtime/lib/* 2>&1
    echo "--- pc-bios (firmware) ---"
    sha256sum "$PKG"/backend/pc-bios/bios-256k.bin \
              "$PKG"/backend/pc-bios/edk2-x86_64-code.fd.bz2 2>&1
    ls -l "$PKG/firmware" 2>&1
    sha256sum "$PKG"/firmware/*.fd 2>&1
} > "$OUT/qemu-identities.txt" 2>&1
say qemu-identities

# ---------------------------------------------------- isolated glibc side ----
{
    echo "prefix: $GLIBC"
    sha256sum "$GLIBC/lib/libc.so.6" "$GLIBC/lib/ld-linux-x86-64.so.2" 2>&1
    echo "--- wrappers ---"
    sha256sum "$WRAP"/qemu-* 2>&1
    for w in "$WRAP"/qemu-patched "$WRAP"/qemu-unpatched; do
        echo "--- $w ---"; cat "$w" 2>&1
    done
} > "$OUT/isolated-runtime.txt" 2>&1
say isolated-runtime

{ "$WRAP/qemu-patched" --version; echo "[exit=$?]"; } \
    > "$OUT/qemu-version.txt" 2>&1
say qemu-version
{ "$WRAP/qemu-patched" -accel help; echo "[exit=$?]"; } \
    > "$OUT/qemu-accel-help.txt" 2>&1
say qemu-accel-help

# --------------------------------------------------------------- rust ------
{
    /opt/rust/prefix/bin/rustc --version --verbose 2>&1
    /opt/rust/prefix/bin/cargo --version 2>&1
    echo "--- installed components ---"
    cat /opt/rust/prefix/lib/rustlib/components 2>&1
    echo "--- bare-metal target libs ---"
    ls -l /opt/rust/prefix/lib/rustlib/x86_64-unknown-none/lib/libcore-*.rlib \
          /opt/rust/prefix/lib/rustlib/x86_64-unknown-uefi/lib/libcore-*.rlib 2>&1
} > "$OUT/rust-cargo-version.txt" 2>&1
say rust-cargo

TMP="$(mktemp -d)"
for target in x86_64-unknown-none x86_64-unknown-uefi; do
    cat > "$TMP/t.rs" <<'EOF'
#![no_std]
#![no_main]
#[panic_handler]
fn p(_: &core::panic::PanicInfo) -> ! { loop {} }
EOF
    {
        echo "target: $target"
        echo "\$ rustc --crate-type staticlib --target $target"
        /opt/rust/prefix/bin/rustc --edition 2021 -C opt-level=2 \
            --crate-type staticlib --target "$target" "$TMP/t.rs" \
            -o "$TMP/t-$target.a" 2>&1
        echo "[rustc exit=$?]"
        ls -l "$TMP/t-$target.a" 2>&1
        sha256sum "$TMP/t-$target.a" 2>&1
    } > "$OUT/rust-target-compile-$target.txt" 2>&1
done
rm -rf "$TMP"
say rust-targets

# ------------------------------------------------------------- compilers ---
mkdir -p "$OUT/compilers"
export ARENA_ZIG="${ARENA_ZIG:-/opt/zig-clang/ziglang/zig}"
{
    gcc --version | head -1
    "$ARENA_ZIG" version
    "$ARENA_ZIG" cc --version | head -3
} > "$OUT/compilers/tool-identities.txt" 2>&1

( cd "$REPO/experiments/c-runtime" && \
  ARENA_ZIG="$ARENA_ZIG" python3 run.py build --cc gcc --cc clang && \
  ARENA_ZIG="$ARENA_ZIG" python3 run.py audit ) \
    > "$OUT/compilers/c-runtime-build-audit.txt" 2>&1
say compilers-build-audit

{
    echo "--- guest ELF hashes ---"
    sha256sum "$REPO"/experiments/c-runtime/build/guest-gcc/*.elf \
              "$REPO"/experiments/c-runtime/build/guest-clang/*.elf 2>&1
    echo "--- static audit verdict ---"
    python3 -c "
import json,sys
s=json.load(open('$REPO/experiments/c-runtime/build/summary.json'))
print('compilers:', s.get('tools'))
print('static_audit_pass:', s['results'].get('static_audit_pass'))
print('failed:', s.get('failed'))
" 2>&1
} > "$OUT/compilers/guest-elf-identities.txt" 2>&1
say compilers-elfs

( cd "$REPO/experiments/c-runtime" && ARENA_ZIG="$ARENA_ZIG" python3 run.py host ) \
    > "$OUT/compilers/host-suite.txt" 2>&1
say host-suite

# ------------------------------------------------- protected build guards ---
mkdir -p "$OUT/guards"
{
    echo "--- Phase-14 PIE guard (unmodified source excerpt) ---"
    sed -n '36,42p' "$REPO/tools/build_phase14_pie.sh"
    echo "--- Phase-14 signed APB1 guard (unmodified source excerpt) ---"
    sed -n '14,18p' "$REPO/tools/build_phase14_bundle.py"
    echo "--- guards are byte-identical to the checkpoint ---"
    git -C "$REPO" diff --stat bfa4e85564a8e7840b862c7fdc89794190a34158 -- \
        tools/build_phase14_pie.sh tools/build_phase14_bundle.py 2>&1
    echo "(empty diff above == unmodified)"
} > "$OUT/guards/guards.txt" 2>&1

BUILD_ROOT=""
if [[ -d /workspace/ArenaOS ]]; then BUILD_ROOT=/workspace/ArenaOS; fi
{
    echo "build_root: ${BUILD_ROOT:-<not mounted>}"
    if [[ -n "$BUILD_ROOT" ]]; then
        ( cd "$BUILD_ROOT/userspace/phase14-pie" && \
          source "$BUILD_ROOT/tools/dev-env/env.sh" && \
          cargo build --release --locked 2>&1 | tail -2 && \
          sha256sum target/x86_64-unknown-none/release/arena-phase14-pie )
        echo "expected_pie: d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"
        ( cd "$BUILD_ROOT" && python3 tools/build_phase14_bundle.py 2>&1 )
        echo "expected_apb1: 04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"
    fi
} > "$OUT/guards/phase14-reproduction.txt" 2>&1
say guards

# ------------------------------------------------------------ backend -----
if [[ -x "$WRAP/qemu-patched" ]]; then
    mkdir -p "$OUT/backend-controls"
    ( cd "$REPO" && python3 tools/backend_controls.py ) \
        > "$OUT/backend-controls/run.txt" 2>&1
    say backend-controls
    for f in "$REPO"/build/backend-controls/*; do
        [[ -f "$f" ]] && cp "$f" "$OUT/backend-controls/" 2>/dev/null
    done
    cp "$REPO"/build/serial-backend-controls-*.log "$OUT/backend-controls/" 2>/dev/null || true
fi

echo "ENV RECEIPT WRITTEN: $OUT"
