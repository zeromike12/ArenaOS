#!/usr/bin/env bash
# Run ALL milestone regression tests (ADR-0005: old tests are never deleted;
# green means every milestone ever completed still completes).
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
failures=0
ran=0

for t in "$REPO_ROOT"/tools/test_m*.py; do
    echo "======================================================================"
    echo "== running $(basename "$t")"
    echo "======================================================================"
    if python3 "$t"; then
        ran=$((ran+1))
    else
        ran=$((ran+1))
        failures=$((failures+1))
        echo "!! $(basename "$t") FAILED"
    fi
done

echo "======================================================================"
if [[ $failures -eq 0 ]]; then
    echo "ALL TESTS PASSED ($ran milestone test scripts)"
    exit 0
else
    echo "$failures of $ran milestone test scripts FAILED"
    exit 1
fi
