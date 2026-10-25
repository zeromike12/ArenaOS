#!/usr/bin/env python3
"""Phase 16 DEVELOPMENT guest runner — NOT QUALIFICATION EVIDENCE.

Runs the unchanged Arena 2 guest suites (guest_c2.py / guest_native.py,
including their exact judges, signed APB1 installs, launch flow, and
exit-status sequences) on a kernel built by tools/build_phase16_dev.sh
(blocker B1: official rust-std components unavailable in this sandbox).
The only substitution is `mtest.build` -> the prebuilt development ESP;
every guest-side assertion is the production harness's own. Results are
labeled DEVELOPMENT in every receipt (docs/phase16/RECOVERY-LOG.md) and
must be re-run on an official build for qualification.

Usage:
  ARENA_QEMU=/path/to/qemu-wrapper ARENA_OVMF_CODE=... ARENA_OVMF_VARS=... \
  ARENA_ZIG=/path/to/zig python3 guest_phase16_dev.py [--suite c2|native]
      [--cc gcc|clang] [--runs N] [--esp build/arena-esp.img]
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

EXP = Path(__file__).resolve().parent
REPO = EXP.parent.parent
sys.path.insert(0, str(EXP))
sys.path.insert(0, str(REPO / "tools"))

import mtest  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--suite", choices=["c2", "native", "fp"], default="c2")
    ap.add_argument("--esp", default=str(REPO / "build/arena-esp.img"))
    ap.add_argument("rest", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    esp = Path(args.esp)
    if not esp.is_file():
        print(f"error: development ESP missing: {esp} (run tools/build_phase16_dev.sh --image)",
              file=sys.stderr)
        return 2
    print(f"PHASE16 DEV RUNNER: suite={args.suite} esp={esp} — NOT QUALIFICATION EVIDENCE")
    mtest.build = lambda label, desktop=False: esp  # noqa: ARG005 - dev-only substitution
    rest = args.rest[1:] if args.rest[:1] == ["--"] else args.rest
    sys.argv = [sys.argv[0]] + rest
    if args.suite == "c2":
        import guest_c2
        return guest_c2.main()
    if args.suite == "fp":
        import guest_fp
        return guest_fp.main()
    import guest_native
    return guest_native.main()


if __name__ == "__main__":
    raise SystemExit(main())
