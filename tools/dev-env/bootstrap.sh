#!/usr/bin/env bash
# ArenaOS dev-environment bootstrap for the offline sandbox (docs/DEV-ENV.md).
#
# Idempotent: every step skips work already done. Installs into /opt (uses
# sudo when not root). On a normal workstation with distro packages
# (qemu-system-x86, ovmf, rustup + uefi/none targets) this script is NOT
# needed — tools/arena_env.py finds system installations automatically.
#
# Pinned inputs (see DEV-ENV.md for why these sources):
#   Rust 1.97.0        PyPI  arena-rust-toolchain[all]==1.97.0
#   Rust library src   GitHub rust-lang/rust tag 1.97.0 (partial clone)
#   QEMU 11.0.2        npm   qemu-portable-linux-x64-musl@0.2.1 (static musl build;
#                        includes EDK2/OVMF firmware blobs in share/qemu)
#   musl 1.2.5         GitHub ifduyue/musl (mirror) — loader/libc for the QEMU build
#   pyfatfs 1.1.0      PyPI  — ESP (FAT16) image creation
set -euo pipefail

RUST_VER="1.97.0"
QEMU_NPM_URL="https://registry.npmjs.org/qemu-portable-linux-x64-musl/-/qemu-portable-linux-x64-musl-0.2.1.tgz"
MUSL_REPO="https://github.com/ifduyue/musl.git"
MUSL_TAG="v1.2.5"
RUST_SRC_REPO="https://github.com/rust-lang/rust.git"

RUST_PREFIX="/opt/rust/prefix"
QEMU_DIR="/opt/qemu"
MUSL_PREFIX="/opt/musl"
BUILD_DIR="/opt/build"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SUDO=""
if [[ $EUID -ne 0 ]]; then SUDO="sudo"; fi

log() { echo "[bootstrap] $*"; }

have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------- directories
$SUDO mkdir -p /opt/rust "$BUILD_DIR"
$SUDO chown -R "$(id -u):$(id -g)" /opt/rust "$BUILD_DIR" 2>/dev/null || true

# ------------------------------------------------------------------- python
log "python packages (zstandard, pyfatfs)"
if ! python3 -c "import zstandard, pyfatfs" 2>/dev/null; then
    pip3 install --user --break-system-packages -q zstandard pyfatfs
fi

# --------------------------------------------------------------------- rust
if [[ -x "$RUST_PREFIX/bin/rustc" ]]; then
    log "rust toolchain present: $("$RUST_PREFIX/bin/rustc" --version)"
else
    log "downloading arena-rust-toolchain wheels ($RUST_VER) from PyPI"
    WHL_DIR="$(mktemp -d)"
    pip3 download --no-deps -q -d "$WHL_DIR" \
        "arena-rust-toolchain==$RUST_VER" \
        "arena-rust-toolchain-data1==$RUST_VER" \
        "arena-rust-toolchain-data2==$RUST_VER" \
        "arena-rust-toolchain-data3==$RUST_VER"
    log "assembling + extracting toolchain archive"
    python3 - "$WHL_DIR" /opt/rust <<'PYEOF'
import os, sys, glob, zipfile
import zstandard, tarfile
whl_dir, target = sys.argv[1], sys.argv[2]
part_files = []
for w in sorted(glob.glob(os.path.join(whl_dir, "*.whl"))):
    with zipfile.ZipFile(w) as z:
        for n in z.namelist():
            base = os.path.basename(n)
            if base.startswith("part_"):
                p = os.path.join(whl_dir, base)
                with z.open(n) as src, open(p, "wb") as dst:
                    dst.write(src.read())
                part_files.append((base, p))
part_files.sort()
assert part_files, "no data parts found"
combined = os.path.join(whl_dir, "toolchain.tar.zst")
with open(combined, "wb") as out:
    for _, p in part_files:
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
print("extracted", len(part_files), "parts to", target)
PYEOF
    rm -rf "$WHL_DIR"
    chmod -R u+w /opt/rust
    "$RUST_PREFIX/bin/rustc" --version
fi

# ------------------------------------------------- bare-metal sysroot crates
if [[ -d "$RUST_PREFIX/lib/rustlib/x86_64-unknown-uefi/lib" && \
      -f "$RUST_PREFIX/lib/rustlib/x86_64-unknown-uefi/lib/libcore.rlib" && \
      -f "$RUST_PREFIX/lib/rustlib/x86_64-unknown-none/lib/libcore.rlib" ]]; then
    log "bare-metal sysroots present (uefi + none)"
else
    if [[ ! -d "$BUILD_DIR/rust-src/library/core" ]]; then
        log "partial-cloning rust-lang/rust @$RUST_VER (library/ only)"
        rm -rf "$BUILD_DIR/rust-src"
        git clone --depth 1 --branch "$RUST_VER" --filter=blob:none --sparse \
            "$RUST_SRC_REPO" "$BUILD_DIR/rust-src"
        ( cd "$BUILD_DIR/rust-src" && git sparse-checkout set library )
    fi
    log "building sysroot crates for bare-metal targets"
    ARENA_RUST_PREFIX="$RUST_PREFIX" ARENA_RUST_SRC="$BUILD_DIR/rust-src" \
        "$REPO_ROOT/tools/build-sysroot.sh"
fi

# --------------------------------------------------------------------- qemu
if [[ -x "$QEMU_DIR/bin/qemu-system-x86_64" && -f "$QEMU_DIR/share/qemu/edk2-x86_64-code.fd" ]]; then
    log "qemu present at $QEMU_DIR"
else
    log "downloading static QEMU (npm tarball) + EDK2 firmware blobs"
    TMP="$(mktemp -d)"
    curl -sL -o "$TMP/qemu.tgz" "$QEMU_NPM_URL"
    mkdir -p "$TMP/x"
    tar xzf "$TMP/qemu.tgz" -C "$TMP/x"
    $SUDO rm -rf "$QEMU_DIR"
    $SUDO cp -r "$TMP/x/package" "$QEMU_DIR"
    $SUDO chown -R "$(id -u):$(id -g)" "$QEMU_DIR"
    rm -rf "$TMP"
fi

# --------------------------------------------------------------------- musl
if [[ -x "$MUSL_PREFIX/lib/libc.so" ]]; then
    log "musl loader present at $MUSL_PREFIX"
else
    log "building musl $MUSL_TAG (dynamic loader for the static-ish QEMU build)"
    if [[ ! -d "$BUILD_DIR/musl" ]]; then
        git clone --depth 1 --branch "$MUSL_TAG" "$MUSL_REPO" "$BUILD_DIR/musl"
    fi
    ( cd "$BUILD_DIR/musl" && ./configure --prefix="$MUSL_PREFIX" --disable-gcc-wrapper >/dev/null \
      && make -j"$(nproc)" >/dev/null && $SUDO make install >/dev/null )
fi

# ------------------------------------------------------------- verification
log "verifying toolchain end-to-end"
"$RUST_PREFIX/bin/rustc" --version
LD_LIBRARY_PATH="$QEMU_DIR/lib" "$MUSL_PREFIX/lib/libc.so" \
    "$QEMU_DIR/bin/qemu-system-x86_64" --version | head -1
python3 -c "import pyfatfs" && echo "[bootstrap] pyfatfs OK"

log "DONE. Next: source tools/dev-env/env.sh && tools/run_tests.sh"
