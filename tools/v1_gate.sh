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
#     controls, both compiler cells of the guest proof and predicate-only
#     refusal matrix, both compiler cells of the real 301/300-page grant
#     integration, ten lifecycle cycles, and the evidence package plus a
#     fresh-extraction verify. That is the review-candidate gate.
#   * full mode adds the Phase-15 behavioral guest check, `tools/run_tests.sh`
#     (the project's 121-suite formal regression), and
#     `tools/stability_loop.sh N` (the final-image boot-stability gate).
#     Only this mode can print a qualification result, and only when every
#     required coverage item is closed or explicitly waived by M.
#   * quick mode is a development convenience and says so.
#
# Reusing an earlier stage is allowed only through `--record` with the same
# identities: code and artifacts unchanged, receipt newer than the newest
# tracked source. Nothing is re-run merely to refresh a timestamp.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

MODE="${V1_GATE_MODE:-guest}"
EVIDENCE="${V1_GATE_EVIDENCE:-docs/v1/evidence/v2-2026-10-30-boundary-r1}"
LOG_DIR="${V1_GATE_LOGS:-build/v1-gate-2026-10-30-boundary-r1}"
RECEIPTS="${V1_GATE_RECEIPTS:-$LOG_DIR/receipts}"
STABILITY_BOOTS="${V1_STABILITY_BOOTS:-100}"
PACKAGING="${EVIDENCE}-packaging"
if [[ -e "$EVIDENCE" || -e "$EVIDENCE.tar.gz" || \
      -e "$EVIDENCE.tar.gz.sha256" || -e "$PACKAGING" || \
      -e "$LOG_DIR" || -e "$RECEIPTS" ]]; then
    echo "refusing to overwrite an existing run/evidence path; choose a fresh V1_GATE_EVIDENCE/V1_GATE_LOGS" >&2
    exit 2
fi
mkdir -p "$LOG_DIR" "$RECEIPTS"

stage_run() { # $1 = stage name; remaining = command
    local name="$1"; shift
    local evidence_dir=""
    case "$name" in
        env) evidence_dir="$EVIDENCE/env" ;;
        guest-*|refusals-*|layout-integration-*) evidence_dir="$EVIDENCE/$name" ;;
        cycles) evidence_dir="$EVIDENCE/lifecycle-cycles" ;;
        package|verify) evidence_dir="$EVIDENCE" ;;
        phase15) evidence_dir="$EVIDENCE/phase15" ;;
    esac
    local evidence_opt=()
    local qualification_opt=()
    if [[ -n "$evidence_dir" ]]; then
        evidence_opt=(--evidence-dir "$evidence_dir")
    fi
    if [[ "${BUILD_QUALIFIED:-1}" != "1" && "$name" != "aggregate-controls" ]]; then
        qualification_opt=(--unqualified)
    fi
    printf '\n=== %s ===\n' "$name"
    python3 tools/v1_gate_stage.py run --stage "$name" --receipts "$RECEIPTS" \
        "${qualification_opt[@]}" "${evidence_opt[@]}" -- "$@"
}

# The Phase-14 PIE guard pins the build path. Build where it is bound.
BUILD_ROOT="$REPO_ROOT"
if [[ -d /workspace/ArenaOS && "${V1_ALLOW_UNPINNED_PATH:-0}" != "1" ]]; then
    BUILD_ROOT=/workspace/ArenaOS
fi

echo "gate mode: $MODE    evidence: $EVIDENCE    receipts: $RECEIPTS"
BUILD_QUALIFIED=1

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
    BUILD_QUALIFIED=0
    echo "canonical build refused (exit $build_rc):"
    grep -E "reproducible PIE fixture hash changed|error:" "$LOG_DIR/build.log" | tail -2
    # Two documented causes, and no third: (1) the official rust-std target
    # components are missing from the private prefix, or (2) the tree is not
    # at the documented build path. The protected fixture embeds the build
    # path in a string, so building the same sources from another directory
    # produces a different digest. Both are recoverable; neither may be
    # recorded as "unavailable" (docs/phase16/dependencies/rust-1.97.0).
    echo "  if PARTS are missing from the prefix: docs/phase16/dependencies/rust-1.97.0/README.md"
    echo "  if PWD is not /workspace/ArenaOS: the protected fixture embeds that path"
    echo "falling back to the documented dev build; receipt is unqualified"
    ( cd "$BUILD_ROOT" && bash tools/build_phase16_dev.sh \
        && python3 tools/espimg.py build/arena-boot.efi build/arena-esp.img ) \
        >> "$LOG_DIR/build.log" 2>&1
    dev_rc=$?
    python3 tools/v1_gate_stage.py record --stage build --receipts "$RECEIPTS" \
        --exit "$dev_rc" --unqualified \
        --note "dev-path build (tools/build_phase16_dev.sh): canonical PIE guard refused (see build.log: components or documented build path)" \
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

stage_run aggregate-controls python3 tools/test_v1_gate_aggregate.py \
    > "$LOG_DIR/aggregate-controls.log" 2>&1
tail -1 "$LOG_DIR/aggregate-controls.log"

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

        rm -rf "$EVIDENCE/layout-integration-$cc"
        stage_run "layout-integration-$cc" env \
            "V1_EVIDENCE_DIR=$EVIDENCE/layout-integration-$cc" \
            V1_CC="$cc" python3 tools/v1_layout_integration.py \
            > "$LOG_DIR/layout-integration-$cc.log" 2>&1
        grep -E "v1-layout-integration.*pass=" \
            "$LOG_DIR/layout-integration-$cc.log" | tail -1
        cp -f "$LOG_DIR/layout-integration-$cc.log" \
            "$EVIDENCE/layout-integration-$cc/harness-console.log"

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
        mkdir -p "$EVIDENCE/phase15"
        stage_run phase15 env \
            "ARENA_PHASE15_EFI=$BUILD_ROOT/build/arena-boot.efi" \
            "ARENA_PHASE15_ESP=$BUILD_ROOT/build/arena-esp.img" \
            python3 tools/test_m15_guest.py > "$LOG_DIR/phase15.log" 2>&1
        grep -E "Phase-15 native guest qualification: (PASS|FAIL)" \
            "$LOG_DIR/phase15.log" | tail -1
        cp -f "$LOG_DIR/phase15.log" "$EVIDENCE/phase15/harness-console.log"
        cp -f build/serial-phase15-avx.log build/serial-phase15-qemu64.log \
            "$EVIDENCE/phase15/" 2>/dev/null || true
        cp -f build/qemu-stderr-phase15-avx.log build/qemu-stderr-phase15-qemu64.log \
            "$EVIDENCE/phase15/" 2>/dev/null || true

        stage_run regression bash tools/run_tests.sh > "$LOG_DIR/regression.log" 2>&1
        grep -E "ALL TESTS PASSED|test suites FAILED" "$LOG_DIR/regression.log" | tail -1
        stage_run stability bash tools/stability_loop.sh "$STABILITY_BOOTS" \
            > "$LOG_DIR/stability.log" 2>&1
        tail -2 "$LOG_DIR/stability.log"
    fi
fi

# Publish every pre-package command, host log and stage receipt inside the
# archive. Package/verify receipts and the final aggregate are published in
# the sibling directory because putting their receipts in the archive would
# make the archive's manifest recursively self-referential.
mkdir -p "$EVIDENCE/gate/receipts" "$EVIDENCE/gate/logs"
for receipt in "$RECEIPTS"/*.json; do
    [[ -f "$receipt" ]] && cp -f "$receipt" "$EVIDENCE/gate/receipts/"
done
for log in "$LOG_DIR"/*.log; do
    [[ -f "$log" ]] && cp -f "$log" "$EVIDENCE/gate/logs/"
done
cat > "$EVIDENCE/gate/commands.txt" <<EOF
cd /workspace/ArenaOS
source tools/dev-env/v1-env.sh
V1_GATE_MODE=$MODE
V1_GATE_EVIDENCE=$EVIDENCE
V1_GATE_LOGS=$LOG_DIR
V1_GATE_RECEIPTS=$RECEIPTS
V1_STABILITY_BOOTS=$STABILITY_BOOTS
bash tools/v1_gate.sh
EOF

stage_run package python3 tools/v1_evidence.py package "$EVIDENCE" \
    > "$LOG_DIR/package.log" 2>&1
tail -1 "$LOG_DIR/package.log"
stage_run verify python3 tools/v1_evidence.py verify "$EVIDENCE.tar.gz" \
    > "$LOG_DIR/verify.log" 2>&1
tail -2 "$LOG_DIR/verify.log"

printf '\n=== gate summary ===\n'
python3 tools/v1_gate_aggregate.py --mode "$MODE" --receipts "$RECEIPTS" \
    | tee "$LOG_DIR/final-aggregate.log"
result=${PIPESTATUS[0]}
mkdir -p "$PACKAGING/gate/receipts" "$PACKAGING/gate/logs"
for receipt in "$RECEIPTS"/*.json; do
    [[ -f "$receipt" ]] && cp -f "$receipt" "$PACKAGING/gate/receipts/"
done
for log in "$LOG_DIR"/*.log; do
    [[ -f "$log" ]] && cp -f "$log" "$PACKAGING/gate/logs/"
done
cp -f "$EVIDENCE.tar.gz.sha256" "$PACKAGING/"
{
    echo "ARENA V1 BOUNDARY EVIDENCE PUBLICATION"
    echo "archive: $(basename "$EVIDENCE").tar.gz"
    echo "external checksum:"
    cat "$EVIDENCE.tar.gz.sha256"
    echo "fresh-retrieval verification:"
    tail -2 "$LOG_DIR/verify.log"
    echo "final aggregate:"
    tail -n 80 "$LOG_DIR/final-aggregate.log"
} > "$PACKAGING/external-receipt.txt"
if [[ $result -ne 0 ]]; then
    echo "logs in $LOG_DIR; receipts in $RECEIPTS; publication in $PACKAGING"
fi
exit $result
