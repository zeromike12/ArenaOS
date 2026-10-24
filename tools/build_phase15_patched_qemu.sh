#!/usr/bin/env bash
set -euo pipefail

# Rebuild the Phase-15 local-TCG backend from QEMU's official v11.1.2 source.
# A rebuilt binary is a new backend identity unless its SHA-256 matches the
# pinned binary in docs/phase15/patched-tcg-qualification.json.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VARIANT="${1:-patched}"
OUT="${2:-$ROOT/build/phase15-qemu-rebuild-$VARIANT}"
[[ "$VARIANT" == patched || "$VARIANT" == unpatched ]] || {
    echo "usage: $0 [patched|unpatched] [fresh-output-directory]" >&2
    exit 2
}
if [[ "$OUT" != /* ]]; then OUT="$ROOT/$OUT"; fi
[[ ! -e "$OUT" ]] || { echo "error: output already exists: $OUT" >&2; exit 1; }
mkdir -p "$OUT/download" "$OUT/source" "$OUT/build"

ARCHIVE="$OUT/download/qemu-official-11.1.2-gitlab.tar.gz"
ARCHIVE_URL="https://gitlab.com/qemu-project/qemu/-/archive/v11.1.2/qemu-v11.1.2.tar.gz"
ARCHIVE_SHA256="fc3d810fc6b27f60df77eaf6748004ca76de2edc2c4ff3a9f9a9ada33066c242"
curl --fail --location --retry 3 --output "$ARCHIVE" "$ARCHIVE_URL"
echo "$ARCHIVE_SHA256  $ARCHIVE" | sha256sum --check --status || {
    echo "error: official QEMU archive hash mismatch" >&2
    exit 1
}
tar -xzf "$ARCHIVE" --strip-components=1 -C "$OUT/source"

if [[ "$VARIANT" == patched ]]; then
    patch --directory="$OUT/source" --strip=1 --input="$ROOT/docs/phase15/experiments/qemu-tcg-xm-prototype/source/qemu-tcg-xm.patch"
fi

printf 'QEMU %s v11.1.2\n' "$VARIANT" > "$OUT/build-identity.txt"
gcc --version | head -n 1 >> "$OUT/build-identity.txt"
meson --version >> "$OUT/build-identity.txt"
ninja --version >> "$OUT/build-identity.txt"
pkg-config --modversion glib-2.0 pixman-1 slirp >> "$OUT/build-identity.txt"

(cd "$OUT/build" && "$OUT/source/configure" \
    --target-list=x86_64-softmmu \
    --enable-tcg \
    --disable-kvm \
    --without-default-features \
    --enable-pixman \
    --enable-slirp \
    --disable-fdt \
    --disable-docs \
    --disable-werror \
    --prefix="$OUT/install") \
    > "$OUT/configure.log" 2>&1
ninja -C "$OUT/build" -j"${PHASE15_JOBS:-$(nproc)}" qemu-system-x86_64 \
    > "$OUT/build.log" 2>&1
cp "$OUT/build/qemu-system-x86_64" "$OUT/qemu-system-x86_64-$VARIANT"
sha256sum "$OUT/qemu-system-x86_64-$VARIANT" | tee "$OUT/binary.sha256"
"$OUT/qemu-system-x86_64-$VARIANT" --version | tee "$OUT/version.txt"
"$OUT/qemu-system-x86_64-$VARIANT" -accel help | tee "$OUT/accelerators.txt"
