#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/build/phase15-independent-xm-controls}"
RUSTC_BIN="${ARENA_RUST_BIN:-}"
if [[ -n "$RUSTC_BIN" ]]; then
    RUSTC="$RUSTC_BIN/rustc"
else
    RUSTC="$(command -v rustc || true)"
fi
if [[ -z "$RUSTC" || ! -x "$RUSTC" ]]; then
    echo "error: rustc not found (set ARENA_RUST_BIN or PATH)" >&2
    exit 1
fi

mkdir -p "$OUT"
"$RUSTC" --edition=2024 --target x86_64-unknown-uefi --crate-type bin \
    -C panic=abort \
    "$ROOT/docs/phase15/experiments/qemu-tcg-xm-prototype/source/controls.rs" \
    -o "$OUT/independent-xm-controls.efi"
PYTHONPATH="$ROOT/build/onboarding/python${PYTHONPATH:+:$PYTHONPATH}" \
    python3 "$ROOT/tools/espimg.py" \
    "$OUT/independent-xm-controls.efi" "$OUT/independent-xm-controls-esp.img"
sha256sum "$OUT/independent-xm-controls.efi" "$OUT/independent-xm-controls-esp.img"
