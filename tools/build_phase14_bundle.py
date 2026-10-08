#!/usr/bin/env python3
"""Rebuild the checked-in, test-root-signed Phase-14 PIE APB1 fixture."""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import apb1_format

IMAGE = ROOT / "userspace/phase14-pie/fixture.elf"
OUTPUT = ROOT / "userspace/phase14-pie/phase14-pie.apb1"
APP_ID = b"org.arenaos.phase14pie"
EXPECTED_SHA256 = "04add54f09042353fc42511978c82f8b8efb87f7d5f0657f9116f456276b3dfa"


def build() -> bytes:
    payload = IMAGE.read_bytes()
    manifest = apb1_format.make_manifest(
        app_id=APP_ID,
        package_id=b"org.arena.editor",
        display_name=b"Phase14 PIE",
        version=14,
        flags=1 | 4,  # MULTI_INSTANCE | HEADLESS
        requested=0,
        entry=b"bin/pie",
        icon=b"",
        width=0,
        height=0,
        associations=(),
    )
    bundle = apb1_format.build_bundle([(1, b"bin/pie", payload)], manifest=manifest)
    parsed = apb1_format.parse_bundle(bundle)
    if parsed.payload != payload or parsed.files[0][1] != b"bin/pie":
        raise ValueError("built APB1 bundle does not contain the exact PIE fixture")
    digest = hashlib.sha256(bundle).hexdigest()
    if digest != EXPECTED_SHA256:
        raise ValueError(f"Phase-14 signed fixture changed unexpectedly: {digest}")
    return bundle


if __name__ == "__main__":
    blob = build()
    OUTPUT.write_bytes(blob)
    print(f"Phase-14 signed APB1: {len(blob)} bytes, SHA-256 {hashlib.sha256(blob).hexdigest()}")
