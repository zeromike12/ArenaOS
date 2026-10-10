#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "$REPO_ROOT/tools/dev-env/env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$REPO_ROOT/tools/dev-env/env.sh"
fi
if [[ -n "${ARENA_RUST_BIN:-}" ]]; then
    export PATH="$ARENA_RUST_BIN:$PATH"
fi

CRATE="$REPO_ROOT/userspace/phase14-pie"
ELF="$CRATE/target/x86_64-unknown-none/release/arena-phase14-pie"
FIXTURE="$CRATE/fixture.elf"

(
    cd "$CRATE"
    cargo build --release --locked
)

if [[ ! -f "$ELF" ]]; then
    echo "error: Rust linker did not produce $ELF" >&2
    exit 1
fi
python3 - "$ELF" "$FIXTURE" <<'PY'
import hashlib
import struct
import sys

path, output = sys.argv[1:]
raw = open(path, "rb").read()
if len(raw) < 64 or raw[:7] != b"\x7fELF\x02\x01\x01":
    raise SystemExit("error: fixture is not ELF64 little-endian")
e_type, machine = struct.unpack_from("<HH", raw, 16)
if e_type != 3 or machine != 62:
    raise SystemExit("error: fixture is not linker-produced x86-64 ET_DYN")
digest = hashlib.sha256(raw).hexdigest()
expected = "d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"
if digest != expected:
    raise SystemExit(f"error: reproducible PIE fixture hash changed: {digest}")
open(output, "wb").write(raw)
print(f"Phase-14 static PIE: {len(raw)} bytes, ELF64 ET_DYN x86-64, SHA-256 {digest}")
PY

echo "fixture: ${FIXTURE#"$REPO_ROOT"/}"
python3 "$REPO_ROOT/tools/build_phase14_bundle.py"
