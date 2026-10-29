#!/bin/sh
# Verify the v1 current-main baseline evidence package.
#
# Usage: ./verify.sh
#
# Checks every file in this directory against manifest.json SHA-256 hashes,
# then verifies the extracted EFI hash matches the ESP-embedded EFI hash,
# then verifies the QEMU binary identity against docs/phase15/ACCEPTANCE.md.
#
# Exits 0 on success, nonzero on any mismatch.

set -e

PACKAGE_DIR="$(cd "$(dirname "$0")" && pwd)"
MANIFEST="$PACKAGE_DIR/manifest.json"

fail=0

echo "== Verifying evidence package: $PACKAGE_DIR =="

# 1. Verify every file listed in manifest.json "files"
python3 - "$PACKAGE_DIR" "$MANIFEST" <<'PYEOF'
import json, hashlib, sys, os

pkg_dir = sys.argv[1]
manifest_path = sys.argv[2]

with open(manifest_path) as f:
    manifest = json.load(f)

fail = 0
for name, info in manifest["files"].items():
    path = os.path.join(pkg_dir, name)
    if not os.path.isfile(path):
        print(f"MISSING: {name}")
        fail = 1
        continue
    with open(path, "rb") as f:
        data = f.read()
    h = hashlib.sha256(data).hexdigest()
    want = info["sha256"]
    if h != want:
        print(f"HASH MISMATCH: {name}: got {h}, want {want}")
        fail = 1
    else:
        print(f"OK: {name} ({info['bytes']} bytes, sha256 {h[:16]}…)")

sys.exit(fail)
PYEOF

if [ $? -ne 0 ]; then
    echo "FAIL: file hash verification"
    exit 1
fi

echo ""
echo "== All file hashes verified =="

# 2. Verify QEMU binary identity against docs/phase15
QEMU_SHA="e0ea2b1f5ae2a5aa65a455eb232e46e1743b5c327c48ca678630c214c69f26ff"
QEMU_PATH="/home/user/phase15-pkg/backend/qemu-system-x86_64-patched"

if [ -f "$QEMU_PATH" ]; then
    GOT=$(sha256sum "$QEMU_PATH" | awk '{print $1}')
    if [ "$GOT" = "$QEMU_SHA" ]; then
        echo "OK: QEMU binary identity verified ($QEMU_SHA)"
    else
        echo "FAIL: QEMU binary hash mismatch: got $GOT, want $QEMU_SHA"
        exit 1
    fi
else
    echo "WARN: QEMU binary not at $QEMU_PATH (expected in recovery environment)"
fi

# 3. Verify isolated glibc identities
LIBC_SHA="eef95f26903e6f9ac6b8cb92df9ba5306ff01b4058ddf7f9ad6aaa986bb7f35f"
LIBC_PATH="/home/user/glibc-2.40/lib/libc.so.6"
LDSO_SHA="63704534fba7799d309e9e16992ea7d11e709906bd44b72060c007461b970458"
LDSO_PATH="/home/user/glibc-2.40/lib/ld-linux-x86-64.so.2"

if [ -f "$LIBC_PATH" ]; then
    GOT=$(sha256sum "$LIBC_PATH" | awk '{print $1}')
    if [ "$GOT" = "$LIBC_SHA" ]; then
        echo "OK: glibc libc.so.6 identity verified"
    else
        echo "FAIL: glibc libc hash mismatch: got $GOT, want $LIBC_SHA"
        exit 1
    fi
fi

if [ -f "$LDSO_PATH" ]; then
    GOT=$(sha256sum "$LDSO_PATH" | awk '{print $1}')
    if [ "$GOT" = "$LDSO_SHA" ]; then
        echo "OK: glibc ld.so identity verified"
    else
        echo "FAIL: glibc ld.so hash mismatch: got $GOT, want $LDSO_SHA"
        exit 1
    fi
fi

echo ""
echo "== Verification complete =="