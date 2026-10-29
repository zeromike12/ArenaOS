#!/usr/bin/env bash
# One command for the V1 gate, with an honest answer about what it proved.
#
#   bash tools/v1_gate.sh                      # guest gate (V1-specific stages)
#   V1_GATE_MODE=full bash tools/v1_gate.sh    # + 121-suite regression and
#                                              #   runtime boot-stability loop
#   V1_GATE_MODE=quick bash tools/v1_gate.sh   # environment + controls only
#
# Every stage writes a receipt (tools/v1_gate_stage.py) stamped with the
# commit, the dirty paths, the EFI/ESP digests and the newest tracked source
# file. `tools/v1_gate_aggregate.py` then decides the result, and it will not
# let a partial run or a stale receipt read as a qualification:
#
#   * guest mode (default) covers build, environment receipts, focused
#     controls, both compiler cells of the guest proof, both refusal-matrix
#     cells, ten lifecycle cycles, and the evidence package plus a
#     fresh-extraction verify. That is the review-candidate gate.
#   * full mode adds `tools/run_tests.sh` (the project's 121-suite formal
#     regression, which is the reason the earlier "reproduces everything"
#     claim was wrong) and `tools/stability_loop.sh N` (the boot-stability
#     gate). Only this mode can print `V1 QUALIFICATION: PASS`.
#   * quick mode is a development convenience and says so.
#
# Reusing an earlier stage is allowed only through `--record` with the same
# identities: code and artifacts unchanged, receipt newer than the newest
# tracked source. Nothing is re-run merely to refresh a timestamp.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

MODE="${V1_GATE_MODE:-guest}"
EVIDENCE="${V1_GATE_EVIDENCE:-docs/v1/evidence/v2-2026-10-29-r2}"
LOG_DIR="${V1_GATE_LOGS:-build/v1-gate}"
RECEIPTS="${V1_GATE_RECEIPTS:-$LOG_DIR/receipts}"
STABILITY_BOOTS="${V1_STABILITY_BOOTS:-100}"
mkdir -p "$LOG_DIR" "$RECEIPTS"

stage_run() { # $1 = stage name; remaining = command
    local name="$1"; shift
    printf '\n=== %s ===\n' "$name"
    python3 tools/v1_gate_stage.py run --stage "$name" --receipts "$RECEIPTS" -- "$@"
}

# The Phase-14 PIE guard pins the build path. Build where it is bound.
BUILD_ROOT="$REPO_ROOT"
if [[ -d /workspace/ArenaOS && "${V1_ALLOW_UNPINNED_PATH:-0}" != "1" ]]; then
    BUILD_ROOT=/workspace/ArenaOS
fi

echo "gate mode: $MODE    evidence: $EVIDENCE    receipts: $RECEIPTS"

# The canonical build is the qualification build. It refuses (correctly)
# when the protected Phase-14 PIE fixture does not reproduce, which in this
# sandbox is caused by blocker B1: the official rust-std target components
# cannot be fetched. Substituting tools/build_phase16_dev.sh keeps the guest
# work executable, and the receipt is marked unqualified so no gate summary
# can present the result as a qualification.
( cd "$BUILD_ROOT" && source tools/dev-env/v1-env.sh >/dev/null 2>&1 || true; \
  bash tools/build.sh --image ) > "$LOG_DIR/build.log" 2>&1
build_rc=$?
if [[ $build_rc -eq 0 ]]; then
    python3 tools/v1_gate_stage.py record --stage build --receipts "$RECEIPTS" \
        --exit 0 --note "canonical build (tools/build.sh --image)" \
        --note "efi $(sha256sum build/arena-boot.efi | cut -d' ' -f1)" > /dev/null
else
    echo "canonical build refused (exit $build_rc):"
    grep -E "reproducible PIE fixture hash changed|error:" "$LOG_DIR/build.log" | tail -2
    echo "falling back to the documented dev build (B1); receipt is unqualified"
    ( cd "$BUILD_ROOT" && bash tools/build_phase16_dev.sh \
        && python3 tools/espimg.py build/arena-boot.efi build/arena-esp.img ) \
        >> "$LOG_DIR/build.log" 2>&1
    dev_rc=$?
    python3 tools/v1_gate_stage.py record --stage build --receipts "$RECEIPTS" \
        --exit "$dev_rc" --unqualified \
        --note "dev-path build (tools/build_phase16_dev.sh): canonical PIE guard blocked by blocker B1 (official rust-std target components unavailable offline)" \
        --note "committed Phase-14 fixture verified, not rebuilt" \
        --note "efi $(sha256sum build/arena-boot.efi | cut -d' ' -f1)" > /dev/null
fi
tail -2 "$LOG_DIR/build.log"
sha256sum "$BUILD_ROOT/build/arena-boot.efi" | tee "$LOG_DIR/boot-efi.sha256"

stage_run env bash tools/v1_env_evidence.sh "$EVIDENCE/env" \
    > "$LOG_DIR/env.log" 2>&1
tail -1 "$LOG_DIR/env.log"

stage_run controls python3 tools/v1_guest_proof.py --controls \
    > "$LOG_DIR/controls.log" 2>&1
tail -1 "$LOG_DIR/controls.log"

if [[ "$MODE" == "quick" ]]; then
    echo
    echo "quick mode: guest stages skipped by request"
else
    source tools/dev-env/v1-env.sh >/dev/null 2>&1 || true

    for cc in gcc clang; do
        rm -rf "$EVIDENCE/guest-$cc"
        stage_run "guest-$cc" env "V1_EVIDENCE_DIR=$EVIDENCE/guest-$cc" \
            V1_CC="$cc" python3 tools/v1_guest_proof.py \
            > "$LOG_DIR/proof-$cc.log" 2>&1
        grep -E "V1 GUEST PROOF (PASS|FAIL)|failed because" "$LOG_DIR/proof-$cc.log" \
            | tail -2
        cp -f build/qemu-stderr-v1-guest-proof.log \
            "$EVIDENCE/guest-$cc/qemu-stderr.log" 2>/dev/null || true
        cp -f "$LOG_DIR/proof-$cc.log" "$EVIDENCE/guest-$cc/harness-console.log"

        rm -rf "$EVIDENCE/refusals-$cc"
        stage_run "refusals-$cc" env "V1_EVIDENCE_DIR=$EVIDENCE/refusals-$cc" \
            V1_CC="$cc" python3 tools/v1_refusals.py \
            > "$LOG_DIR/refusals-$cc.log" 2>&1
        grep -E "cases=.*(PASS|FAIL)" "$LOG_DIR/refusals-$cc.log" | tail -1
        cp -f "$LOG_DIR/refusals-$cc.log" "$EVIDENCE/refusals-$cc/harness-console.log"

        # Scratch inputs, not evidence: their digests are in evidence.json.
        rm -f "$EVIDENCE/guest-$cc"/arena-disk.img "$EVIDENCE/guest-$cc/BOOTX64.EFI"
        rm -f "$EVIDENCE/refusals-$cc"/arena-disk.img "$EVIDENCE/refusals-$cc/BOOTX64.EFI"
    done

    rm -rf "$EVIDENCE/lifecycle-cycles"
    stage_run cycles env "V1_EVIDENCE_DIR=$EVIDENCE/lifecycle-cycles" \
        V1_CYCLES=10 python3 tools/v1_cycles.py > "$LOG_DIR/cycles.log" 2>&1
    grep -E "cycles PASS" "$LOG_DIR/cycles.log" | tail -1
    cp -f build/qemu-stderr-v1-cycles.log \
        "$EVIDENCE/lifecycle-cycles/qemu-stderr.log" 2>/dev/null || true
    cp -f "$LOG_DIR/cycles.log" "$EVIDENCE/lifecycle-cycles/harness-console.log"
    rm -f "$EVIDENCE/lifecycle-cycles"/arena-disk*.img \
          "$EVIDENCE/lifecycle-cycles/BOOTX64.EFI"

    if [[ "$MODE" == "full" ]]; then
        stage_run regression bash tools/run_tests.sh > "$LOG_DIR/regression.log" 2>&1
        grep -E "ALL TESTS PASSED|test suites FAILED" "$LOG_DIR/regression.log" | tail -1
        stage_run stability bash tools/stability_loop.sh "$STABILITY_BOOTS" \
            > "$LOG_DIR/stability.log" 2>&1
        tail -2 "$LOG_DIR/stability.log"
    fi
fi

stage_run package python3 tools/v1_evidence.py package "$EVIDENCE" \
    > "$LOG_DIR/package.log" 2>&1
tail -1 "$LOG_DIR/package.log"
stage_run verify python3 tools/v1_evidence.py verify "$EVIDENCE.tar.gz" \
    > "$LOG_DIR/verify.log" 2>&1
tail -2 "$LOG_DIR/verify.log"

printf '\n=== gate summary ===\n'
python3 tools/v1_gate_aggregate.py --mode "$MODE" --receipts "$RECEIPTS"
result=$?
if [[ $result -ne 0 ]]; then
    echo "logs in $LOG_DIR; receipts in $RECEIPTS"
fi
exit $result
