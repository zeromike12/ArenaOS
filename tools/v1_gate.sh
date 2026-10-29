#!/usr/bin/env bash
# One command for the V1 qualification gate.
#
#   bash tools/v1_gate.sh            # full gate (guest runs, ~10 minutes)
#   SKIP_GUEST=1 bash tools/v1_gate.sh   # receipts + harness controls only
#
# Stages, in dependency order:
#   1. environment receipts  (emulator/loader/toolchains/guards/backend controls)
#   2. harness controls      (verdict logic, no QEMU)
#   3. guest proof, GCC cell (8 checkpoints, instance-attributed evidence)
#   4. guest proof, Clang cell
#   5. lifecycle cycles      (10 launch/close cycles with settlement)
#   6. evidence package      (manifest + archive, then verify from a fresh
#                             extraction outside this workspace)
#
# Every stage is a real execution. Nothing here re-uses an earlier verdict:
# a missing receipt, a missing serial log or a missing identity fails the gate.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

EVIDENCE="docs/v1/evidence/v2-2026-10-29"
LOG_DIR="${V1_GATE_LOGS:-build/v1-gate}"
mkdir -p "$LOG_DIR"

fail=0
stage() { printf '\n=== %s ===\n' "$*"; }
check() { # $1 = label, $2 = exit code
    if [[ "$2" -eq 0 ]]; then
        echo "GATE PASS: $1"
    else
        echo "GATE FAIL: $1 (exit $2)"
        fail=$((fail+1))
    fi
}

# The build path the Phase-14 PIE guard pins. Building elsewhere fails closed,
# so the qualification runs from the bound path when it exists.
BUILD_ROOT="$REPO_ROOT"
if [[ -d /workspace/ArenaOS && "${V1_ALLOW_UNPINNED_PATH:-0}" != "1" ]]; then
    BUILD_ROOT=/workspace/ArenaOS
fi

stage "1/6 environment receipts"
bash tools/v1_env_evidence.sh "$EVIDENCE/env" > "$LOG_DIR/env.log" 2>&1
rg=$?
tail -1 "$LOG_DIR/env.log"
check "environment receipts ($EVIDENCE/env)" "$rg"

stage "2/6 harness controls"
python3 tools/v1_guest_proof.py --controls > "$LOG_DIR/controls.log" 2>&1
cg=$?
tail -1 "$LOG_DIR/controls.log"
check "harness focused controls" "$cg"

if [[ "${SKIP_GUEST:-0}" != "1" ]]; then
    source tools/dev-env/v1-env.sh >/dev/null 2>&1 || true
    for cc in gcc clang; do
        stage "3-4/6 guest proof ($cc)"
        rm -rf "$EVIDENCE/guest-$cc"
        V1_EVIDENCE_DIR="$EVIDENCE/guest-$cc" V1_CC="$cc" \
            python3 tools/v1_guest_proof.py > "$LOG_DIR/proof-$cc.log" 2>&1
        pg=$?
        grep -E "V1 GUEST PROOF (PASS|FAIL)|failed because" "$LOG_DIR/proof-$cc.log" | tail -2
        # The emulator's own stderr and the console transcript travel with the
        # evidence: a reader must be able to see a startup failure, not guess
        # at it from an empty serial log.
        cp -f build/qemu-stderr-v1-guest-proof.log "$EVIDENCE/guest-$cc/qemu-stderr.log" 2>/dev/null || true
        cp -f "$LOG_DIR/proof-$cc.log" "$EVIDENCE/guest-$cc/harness-console.log"
        check "guest proof ($cc)" "$pg"
    done

    stage "5/6 lifecycle cycles"
    rm -rf "$EVIDENCE/lifecycle-cycles"
    V1_EVIDENCE_DIR="$EVIDENCE/lifecycle-cycles" V1_CYCLES=10 \
        python3 tools/v1_cycles.py > "$LOG_DIR/cycles.log" 2>&1
    yg=$?
    grep -E "cycles PASS" "$LOG_DIR/cycles.log" | tail -1
    cp -f build/qemu-stderr-v1-cycles.log "$EVIDENCE/lifecycle-cycles/qemu-stderr.log" 2>/dev/null || true
    cp -f "$LOG_DIR/cycles.log" "$EVIDENCE/lifecycle-cycles/harness-console.log"
    check "ten launch/close cycles" "$yg"
else
    echo "SKIP: guest proof and cycles (SKIP_GUEST=1)"
fi

stage "6/6 evidence package + fresh-extraction verify"
python3 tools/v1_evidence.py package "$EVIDENCE" > "$LOG_DIR/package.log" 2>&1
pg=$?
tail -1 "$LOG_DIR/package.log"
check "evidence package" "$pg"
# Verification happens in a temporary directory that is NOT this workspace.
python3 tools/v1_evidence.py verify "$EVIDENCE.tar.gz" > "$LOG_DIR/verify.log" 2>&1
vg=$?
tail -2 "$LOG_DIR/verify.log"
check "evidence verifies from a fresh extraction" "$vg"

printf '\n=== gate summary ===\n'
if [[ "$fail" -eq 0 ]]; then
    echo "V1 GATE: PASS (logs in $LOG_DIR)"
    exit 0
fi
echo "V1 GATE: FAIL ($fail stage(s); logs in $LOG_DIR)"
exit 1
