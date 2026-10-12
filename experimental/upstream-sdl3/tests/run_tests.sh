#!/usr/bin/env bash
# ==============================================================================
# run_tests.sh: Run Host Safety Unit Tests for Freestanding Upstream SDL3 Runtime
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UPSTREAM_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "=== Running ArenaOS Freestanding Allocator Tests ==="
gcc -O2 -pthread -Wall -Wextra -DARENAOS_HOST_TEST \
    "${SCRIPT_DIR}/test_allocator.c" \
    "${UPSTREAM_ROOT}/SDL-release-3.2.0/src/platform/arenaos/arenaos_memory.c" \
    -o /tmp/test_allocator
/tmp/test_allocator
rm -f /tmp/test_allocator

echo ""
echo "=== Running Canonical ARST v2 Startup Gate Tests ==="
gcc -O2 -Wall -Wextra -DARENAOS_HOST_TEST \
    "${SCRIPT_DIR}/test_startup.c" \
    "${UPSTREAM_ROOT}/SDL-release-3.2.0/src/platform/arenaos/arenaos_entry.c" \
    -o /tmp/test_startup
/tmp/test_startup
rm -f /tmp/test_startup

echo ""
echo "=== Running ArenaOS Timer & Notification Rights Contract Tests ==="
gcc -O2 -Wall -Wextra \
    "${SCRIPT_DIR}/test_timer_contract.c" \
    -o /tmp/test_timer
/tmp/test_timer
rm -f /tmp/test_timer

echo ""
echo "=== Running ArenaOS IEEE-754 Safe Bitwise Conversion Tests ==="
gcc -O2 -Wall -Wextra \
    "${SCRIPT_DIR}/test_ieee754.c" \
    -o /tmp/test_ieee754
/tmp/test_ieee754
rm -f /tmp/test_ieee754

echo ""
echo "ALL HOST TESTS COMPLETED SUCCESSFULLY!"
