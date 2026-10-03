#!/usr/bin/env bash
# Run ALL milestone regression tests (ADR-0005: old tests are never deleted;
# green means every milestone ever completed still completes).
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Match build.sh: host-side suites also need the repository's Rust toolchain.
if [[ -f "$REPO_ROOT/tools/dev-env/env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$REPO_ROOT/tools/dev-env/env.sh"
fi
failures=0
ran=0
source_commit=$(git -C "$REPO_ROOT" rev-parse HEAD)
source_status=$(git -C "$REPO_ROOT" status --porcelain)
echo "QUALIFICATION SOURCE COMMIT: $source_commit"
if [[ -z "$source_status" ]]; then
    echo "QUALIFICATION SOURCE CLEAN: yes"
else
    echo "QUALIFICATION SOURCE CLEAN: no (development run)"
fi

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

echo "== host-side TCP wire/timeout policy tests"
if (cd "$REPO_ROOT" && rustc --test --edition 2024 \
    userspace/netstackd/src/tcp.rs -o build/tcp-parser-tests && build/tcp-parser-tests); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! host-side TCP parser tests FAILED"
fi

# M7.7: exercise the actual no_std public networking API against a
# strict fake transport; QEMU tests separately prove real packets.
echo "== host-side networking API contract tests"
if (cd "$REPO_ROOT" && rustc --test --edition 2024 tools/net_api_test.rs \
    -o build/net-api-tests && build/net-api-tests); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! host-side networking API tests FAILED"
fi

# ADR-0052: the independently linked userspace rlib is tested on the
# host AND compiled for the actual bare-metal guest target. The guest
# fixture exercises two separate FS and two separate network consumers.
echo "== host and no_std target Phase 8.3 userspace client libraries"
if (cd "$REPO_ROOT" && cargo test --manifest-path userspace/arena-lib/Cargo.toml \
    --lib --target x86_64-unknown-linux-gnu && \
    cargo build --manifest-path userspace/arena-lib/Cargo.toml \
    --release --target x86_64-unknown-none); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! linked no_std client libraries FAILED"
fi

# Phase 8.0 groundwork: fail-closed no_std manifest, caller-cap
# inventory and readiness badge gate. Ring-3 syscall proof runs in
# the M4 shell fixture; real manager boot in test_m8_bootstrap.py.
echo "== host-side service manifest and inventory policy tests"
if (cd "$REPO_ROOT" && rustc --test --edition 2024 \
    userspace/servicemgr/src/lib.rs -o build/service-manifest-tests \
    && build/service-manifest-tests && rustc --crate-type lib \
    --edition 2024 -D warnings userspace/servicemgr/src/lib.rs \
    -o build/libarena-service-manifest.rlib); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! service manifest policy tests FAILED"
fi

# ADR-0046 foundation: test the exact no_std codec/scanner against
# independent Python reference vectors, compile it for the bare-metal
# target and retain the AFS1 fallback-limit probe. No guest configd yet.
echo "== host-side Phase 8.1 config record and AFS1 integrity-limit probes"
if (cd "$REPO_ROOT" && rustc --test --edition 2024 -D warnings \
    userspace/config.rs -o build/config-record-tests && build/config-record-tests && \
    rustc --crate-type lib --edition 2024 -D warnings \
    --target x86_64-unknown-none --emit=metadata \
    tools/config_no_std.rs -o build/config-no-std.rmeta && \
    python3 tools/test_config_record.py && \
    python3 tools/probe_config_commit_fallback.py); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! host-side configuration record tests FAILED"
fi

# ADR-0048 completed persistence path: independent codec/reference,
# fail-closed reserved namespace and no_std bare-metal compilation.
# Guest crash/restart/refusal fixtures are separate test_m82_policy_*.py.
echo "== host-side Phase 8.2 immutable permission decision record"
if (cd "$REPO_ROOT" && rustc --test --edition 2024 -D warnings \
    userspace/permission.rs -o build/permission-record-tests && build/permission-record-tests && \
    rustc --crate-type lib --edition 2024 -D warnings \
    --target x86_64-unknown-none --emit=metadata \
    tools/permission_no_std.rs -o build/permission-no-std.rmeta && \
    python3 tools/test_permission_record.py); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! permission policy record tests FAILED"
fi

# ADR-0048: source-derived static bounds on both host and bare-metal
# x86_64; the separate guest suite checks live capacity and resources.
echo "== host and no_std target cap/IPC layout audit"
if (cd "$REPO_ROOT" && python3 tools/probe_capspace_layout.py); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! Phase 8.2 cap/IPC layout audit FAILED"
fi

# ADR-0054 accepted persistent-record byte codec (not fsd/guest proof).
echo "== host-independent Phase 8.5 AINS/AACT records and no_std compile"
if (cd "$REPO_ROOT" && python3 tools/test_phase85_records.py && \
    rustc --crate-type lib --edition 2024 -D warnings \
    --target x86_64-unknown-none --emit=metadata \
    tools/installed_no_std.rs -o build/installed-no-std.rmeta); then
    ran=$((ran+1))
else
    ran=$((ran+1))
    failures=$((failures+1))
    echo "!! Phase 8.5 records/refusal tests FAILED"
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
if [[ "$(git -C "$REPO_ROOT" rev-parse HEAD)" != "$source_commit" || \
      "$(git -C "$REPO_ROOT" status --porcelain)" != "$source_status" ]]; then
    failures=$((failures+1))
    echo "!! source changed during full suite"
fi
echo "QUALIFICATION SOURCE END: $(git -C "$REPO_ROOT" rev-parse HEAD)"
if [[ $failures -eq 0 ]]; then
    echo "ALL TESTS PASSED ($ran test suites)"
    exit 0
else
    echo "$failures of $ran test suites FAILED"
    exit 1
fi
