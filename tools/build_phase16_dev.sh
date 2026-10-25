#!/usr/bin/env bash
# Phase 16 DEVELOPMENT build — NOT A QUALIFICATION BUILD.
#
# Why this exists (docs/phase16/RECOVERY-LOG.md, blocker B1): the official
# tools/build.sh rebuilds the protected Phase 14 PIE fixture and refuses
# (correctly) when the rebuild does not reproduce its pinned SHA-256. That
# reproduction requires the official rust-std target components, which this
# sandbox cannot download. Without any kernel build, no C guest integration
# could be exercised at all.
#
# What this script does differently — exactly one step: instead of running
# tools/build_phase14_pie.sh, it verifies that the COMMITTED fixture
# (userspace/phase14-pie/fixture.elf, embedded by the kernel via
# include_bytes!) still hashes to the protected identity d7b0a8cf..., and
# refuses otherwise. tools/build.sh, tools/build_phase14_pie.sh, the guard,
# and every expected hash are UNCHANGED. All other inputs (source-built
# unknown-none/uefi sysroot libraries) make the resulting EFI/ESP
# bit-different from any qualified artifact.
#
# Therefore: artifacts from this script may be used for development guest
# runs and debugging ONLY. They must never be presented as qualification
# evidence, stability receipts, release candidates, or Phase 14/15 identity
# reproductions. The APB1 bundle step is also skipped (it belongs to the
# guarded rebuild); the committed phase14-pie.apb1 is used as-is.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE="$REPO_ROOT/userspace/phase14-pie/fixture.elf"
EXPECTED="d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e"

echo "========================================================================" >&2
echo "WARNING: Phase 16 DEVELOPMENT BUILD — artifacts are NOT qualified." >&2
echo "The protected Phase 14 fixture below is verified, not rebuilt (B1)." >&2
echo "========================================================================" >&2

ACTUAL="$(sha256sum "$FIXTURE" | cut -d' ' -f1)"
if [[ "$ACTUAL" != "$EXPECTED" ]]; then
    echo "error: committed Phase 14 fixture identity mismatch: $ACTUAL" >&2
    exit 1
fi
echo "Phase 14 fixture (committed, verified not rebuilt): $EXPECTED"

# Copy of tools/build.sh from the step AFTER build_phase14_pie.sh. Kept by
# extraction so this file cannot drift silently: regenerate with
#   sed -n '/^# Phase 15:/,$p' tools/build.sh
# and compare before trusting a fresh checkout.
# build.sh derives REPO_ROOT from its own location (tools/); build/ is also
# one level below the root, so the extracted script resolves identically.
mkdir -p "$REPO_ROOT/build"
tmp="$REPO_ROOT/build"
python3 - "$REPO_ROOT/tools/build.sh" "$tmp/build-rest.sh" <<'PY'
import sys
src = open(sys.argv[1]).read()
marker = 'echo "Phase-15 fixture:'
# Take everything from the Phase-15 fixture comment block onward.
i = src.index("# Phase 15:")
tail = src[i:]
# Preserve the original header (env/profile setup) up to the payload step.
j = src.index("# M4.1 (ADR-0016)")
head = src[:j]
k = src.index("# Phase 14: the genuine")
payload = src[j:k]
open(sys.argv[2], "w").write(head + payload + tail)
PY
bash "$tmp/build-rest.sh" "$@"
echo "DEVELOPMENT BUILD COMPLETE — NOT QUALIFIED (B1). EFI/ESP above are dev-only." >&2
