#!/usr/bin/env bash
# Run ALL milestone regression tests (ADR-0005: old tests are never deleted;
# green means every milestone ever completed still completes).
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
failures=0
ran=0

# Host-side unit tests (ROADMAP 2.5+): pure-logic crates run natively.
for pkg in arena-heap arena-sync; do
    echo "======================================================================"
    echo "== host-side unit tests ($pkg: cargo test --lib)"
    echo "======================================================================"
    if (cd "$REPO_ROOT/kernel" && cargo test -p "$pkg" --lib --target x86_64-unknown-linux-gnu); then
        ran=$((ran+1))
    else
        ran=$((ran+1))
        failures=$((failures+1))
        echo "!! host-side unit tests ($pkg) FAILED"
    fi
done

# M7.5: the bounded DNS codec is pure Rust, so exercise hostile reply
# shapes on the host as well as real slirp traffic in the boot gate.
echo "== host-side DNS parser tests"
if (cd "$REPO_ROOT" && mkdir -p build && rustc --test --edition 2024 \
    userspace/netstackd/src/dns.rs -o build/dns-parser-tests && build/dns-parser-tests); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! host-side DNS parser tests FAILED"
fi

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
    echo "ALL TESTS PASSED ($ran test suites)"
    exit 0
else
    echo "$failures of $ran test suites FAILED"
    exit 1
fi
