#!/usr/bin/env bash
# Run the formal regression suite, then restore the exact canonical candidate.
# Individual suites deliberately build alternate graphics/mutation images;
# full-gate stability and receipts must describe the production desktop EFI.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

if [[ $# -ne 3 ]]; then
    echo "usage: $0 EXPECTED_EFI_SHA256 EXPECTED_ESP_SHA256 RESTORE_BUILD_LOG" >&2
    exit 2
fi
EXPECTED_EFI_SHA256="$1"
EXPECTED_ESP_SHA256="$2"
RESTORE_BUILD_LOG="$3"
if [[ ! "$EXPECTED_EFI_SHA256" =~ ^[0-9a-f]{64}$ ||
      ! "$EXPECTED_ESP_SHA256" =~ ^[0-9a-f]{64}$ ]]; then
    echo "error: expected EFI and ESP SHA-256 identities are required" >&2
    exit 2
fi

suite_rc=0
bash tools/run_tests.sh || suite_rc=$?
echo "[v1-regression-restore] 121-suite command exit=$suite_rc"

mkdir -p "$(dirname "$RESTORE_BUILD_LOG")"
restore_rc=0
env -u ARENA_GRAPHICS_FIXTURE bash tools/build.sh --image \
    > "$RESTORE_BUILD_LOG" 2>&1 || restore_rc=$?
if [[ $restore_rc -ne 0 ]]; then
    echo "[v1-regression-restore] canonical production rebuild failed (exit $restore_rc); see $RESTORE_BUILD_LOG"
else
    actual_efi_sha256="$(sha256sum build/arena-boot.efi | awk '{print $1}')"
    actual_esp_sha256="$(sha256sum build/arena-esp.img | awk '{print $1}')"
    profile="MISSING"
    if [[ -f build/graphics-profile.txt ]]; then
        profile="$(cat build/graphics-profile.txt)"
    fi
    echo "[v1-regression-restore] profile=$profile EFI=$actual_efi_sha256 ESP=$actual_esp_sha256"
    if [[ "$profile" != "desktop" ]]; then
        echo "[v1-regression-restore] ERROR: canonical rebuild did not select the"
        echo "[v1-regression-restore] production desktop profile"
        restore_rc=1
    fi
    if [[ "$actual_efi_sha256" != "$EXPECTED_EFI_SHA256" ||
          "$actual_esp_sha256" != "$EXPECTED_ESP_SHA256" ]]; then
        echo "[v1-regression-restore] ERROR: post-regression candidate differs"
        echo "[v1-regression-restore] from the candidate tested before the suite"
        echo "[v1-regression-restore] expected EFI=$EXPECTED_EFI_SHA256 ESP=$EXPECTED_ESP_SHA256"
        restore_rc=1
    fi
    if [[ $restore_rc -eq 0 ]]; then
        echo "[v1-regression-restore] canonical candidate restoration PASS"
    fi
fi

if [[ $suite_rc -ne 0 ]]; then
    exit "$suite_rc"
fi
exit "$restore_rc"
