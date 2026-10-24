#!/usr/bin/env bash
# Run the full historical suite on the explicit Phase-15 diagnostic image.
# The ordinary tools/run_tests.sh invocation remains strict and still runs
# the dedicated #XF acceptance test.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
unset ARENA_PHASE15_OSXSAVE_SEED
exec bash "$REPO_ROOT/tools/run_tests.sh" --historical-diagnostic
