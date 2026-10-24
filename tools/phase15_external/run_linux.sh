#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! command -v python3 >/dev/null 2>&1; then
  echo "BLOCKED: Python 3.10+ is required; install it from your Linux distribution's official repository." >&2
  exit 3
fi
if ! python3 -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)'; then
  echo "BLOCKED: Python 3.10+ is required; found $(python3 --version 2>&1)." >&2
  exit 3
fi
exec python3 "$SCRIPT_DIR/run_external.py" --backend kvm "$@"
