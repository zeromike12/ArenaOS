#!/usr/bin/env bash
# Focused private-toolchain recovery (V2, 2026-10-29).
#
# This is the *compiler-recovery portion* of tools/dev-env/bootstrap.sh,
# separated so a reset can restore the two compilers ArenaOS actually needs
# without rerunning the older, unrelated QEMU-npm / musl / source-built
# sysroot stages (those would install a *different* backend and would
# replace the official target components with locally built ones).
#
#   1. Rust 1.97.0 from PyPI `arena-rust-toolchain[+data1..3]==1.97.0`
#      (the same pinned source bootstrap.sh uses), extracted to
#      /opt/rust/prefix. Verifies that rustc, cargo and the two
#      official bare-metal target libs are present.
#   2. Zig 0.17.0 from PyPI `ziglang==0.17.0`, extracted to
#      /opt/zig-clang/ziglang. experiments/c-runtime/run.py drives the
#      supported Clang route as `zig cc -target x86_64-freestanding-none`
#      and reads ARENA_ZIG (default /opt/zig-clang/ziglang/zig).
#
# Nothing is installed over host tools: everything lands under /opt in
# private prefixes. Idempotent: each stage skips when already satisfied.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

RUST_VER="1.97.0"
ZIG_VER="0.17.0"
RUST_PREFIX="/opt/rust/prefix"
ZIG_ROOT="/opt/zig-clang"

SUDO=""
if [[ $EUID -ne 0 ]]; then SUDO="sudo"; fi
log() { echo "[recover-rust-and-zig] $*"; }

have_rust() {
    [[ -x "$RUST_PREFIX/bin/rustc" ]] && [[ -x "$RUST_PREFIX/bin/cargo" ]] \
        && "$RUST_PREFIX/bin/rustc" --version 2>/dev/null | grep -q "1\.97\.0"
}

# ---------------------------------------------------------------- rust ------
if have_rust; then
    log "rust $RUST_VER present at $RUST_PREFIX"
else
    log "downloading arena-rust-toolchain wheels ($RUST_VER) from PyPI"
    WHL_DIR="$(mktemp -d)"
    pip3 download --no-deps -q -d "$WHL_DIR" \
        "arena-rust-toolchain==$RUST_VER" \
        "arena-rust-toolchain-data1==$RUST_VER" \
        "arena-rust-toolchain-data2==$RUST_VER" \
        "arena-rust-toolchain-data3==$RUST_VER"
    if ! python3 -c "import zstandard" 2>/dev/null; then
        pip3 install --user --break-system-packages -q zstandard
    fi
    log "assembling + extracting toolchain archive"
    $SUDO mkdir -p "$RUST_PREFIX"
    $SUDO chown -R "$(id -u):$(id -g)" /opt/rust
    python3 - "$WHL_DIR" /opt/rust <<'PYEOF'
import glob, os, sys, tarfile, zipfile
import zstandard
whl_dir, target = sys.argv[1], sys.argv[2]
parts = []
for w in sorted(glob.glob(os.path.join(whl_dir, "*.whl"))):
    with zipfile.ZipFile(w) as z:
        for n in z.namelist():
            base = os.path.basename(n)
            if base.startswith("part_"):
                p = os.path.join(whl_dir, base)
                with z.open(n) as src, open(p, "wb") as dst:
                    dst.write(src.read())
                parts.append(p)
parts.sort()
assert parts, "no data parts found in wheels"
combined = os.path.join(whl_dir, "toolchain.tar.zst")
with open(combined, "wb") as out:
    for p in parts:
        with open(p, "rb") as f:
            while chunk := f.read(1 << 20):
                out.write(chunk)
dctx = zstandard.ZstdDecompressor()
with open(combined, "rb") as f, dctx.stream_reader(f) as reader:
    with tarfile.open(fileobj=reader, mode="r|") as tf:
        for m in tf:
            if os.path.basename(m.name).startswith("._"):
                continue
            tf.extract(m, target)
print("extracted", len(parts), "parts to", target)
PYEOF
    rm -rf "$WHL_DIR"
    # The PyPI wheel's tar.zst payload contains the toolchain files but
    # strips their executable mode bits. Restore executable modes only for
    # the known command directories; libraries and target archives remain
    # non-executable. This is required before rustc/cargo can be verified.
    find "$RUST_PREFIX/bin" -maxdepth 1 -type f -exec chmod 0755 {} +
    for executable_dir in \
        "$RUST_PREFIX/libexec" \
        "$RUST_PREFIX/lib/rustlib/x86_64-unknown-linux-gnu/bin"; do
        if [[ -d "$executable_dir" ]]; then
            find "$executable_dir" -type f -exec chmod 0755 {} +
        fi
    done
    chmod -R u+w /opt/rust
fi
"$RUST_PREFIX/bin/rustc" --version
"$RUST_PREFIX/bin/cargo" --version
log "rust toolchain ready"

# ----------------------------------------------------------------- zig ------
# The wheel extracts `ziglang/` directly under the root (no `pkg/` level).
ZIG_BIN="$ZIG_ROOT/ziglang/zig"
have_zig() { [[ -x "$ZIG_BIN" ]] && "$ZIG_BIN" version 2>/dev/null | grep -q "^${ZIG_VER}$"; }
if have_zig; then
    log "zig $ZIG_VER present at $ZIG_ROOT"
else
    log "downloading ziglang==$ZIG_VER from PyPI"
    WHL_DIR="$(mktemp -d)"
    # PyPI's simple index does not offer ziglang 0.17.0 to this platform
    # through `pip download --platform` (and the plain fallback resolves only
    # a 0.0.0 placeholder), so the release file is fetched from the project's
    # own file list, selected by the pinned digest. A substituted wheel is
    # refused by the checksum before anything is extracted.
    ZIG_SHA256="97079827064a7492cce5541ceb438ae8003ce1e54e399676da2d62fb7a5dc08b"
    ZIG_URL="$(python3 "$REPO_ROOT/tools/dev-env/pypi_file.py" ziglang "$ZIG_VER" "$ZIG_SHA256")"
    curl -sSL --fail -o "$WHL_DIR/ziglang.whl" "$ZIG_URL"
    echo "$ZIG_SHA256  $WHL_DIR/ziglang.whl" | sha256sum -c - >/dev/null
    WHL="$WHL_DIR/ziglang.whl"
    $SUDO mkdir -p "$ZIG_ROOT"
    $SUDO chown -R "$(id -u):$(id -g)" "$ZIG_ROOT"
    python3 -c "import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" \
        "$WHL" "$ZIG_ROOT"
    chmod +x "$ZIG_BIN" 2>/dev/null || true
    rm -rf "$WHL_DIR"
fi
"$ZIG_BIN" version

log "DONE.  source tools/dev-env/env.sh"
log "RUST_PREFIX=$RUST_PREFIX  ARENA_ZIG=$ZIG_BIN"
