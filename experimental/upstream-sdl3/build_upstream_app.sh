#!/usr/bin/env bash
# Build script for linking genuine upstream SDL 3.2.0 demo binary on ArenaOS
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
UPSTREAM_DIR="${SCRIPT_DIR}/SDL-release-3.2.0"
BUILD_DIR="${SCRIPT_DIR}/build"

echo "=== Compiling & Linking Genuine Upstream SDL 3.2.0 Application ==="

# 1. Ensure static library is built
if [ ! -f "${BUILD_DIR}/libSDL3_upstream.a" ]; then
    bash "${SCRIPT_DIR}/build_upstream_arenaos.sh"
fi

FLAGS="-I${UPSTREAM_DIR}/include -I${UPSTREAM_DIR}/src -I${UPSTREAM_DIR}/include/build_config \
-Os -ffreestanding -fno-builtin -nostdlib -fno-pie -no-pie \
-ffunction-sections -fdata-sections \
-include ${UPSTREAM_DIR}/include/build_config/SDL_build_config_arenaos.h \
-DSDL_STATIC_LIB=1 \
-DSDL_PLATFORM_PRIVATE=1 \
-DSDL_PLATFORM_PRIVATE_NAME=\"ArenaOS\""

LDFLAGS="-Os -ffreestanding -nostdlib -static -fno-pie -no-pie \
-Wl,--gc-sections -Wl,-e,_start -Wl,-Ttext=0x200000 -Wl,--build-id=none"

# 2. Compile demo main
gcc $FLAGS -c "${SCRIPT_DIR}/src/demo/main.c" -o "${BUILD_DIR}/demo_main.o"

# 3. Link executable
gcc $LDFLAGS \
    "${ROOT_DIR}/experimental/sdl3/src/platform/arenaos/arenaos_entry.c" \
    "${ROOT_DIR}/experimental/sdl3/src/platform/arenaos/arenaos_client.c" \
    "${ROOT_DIR}/experimental/sdl3/src/platform/arenaos/arenaos_memory.c" \
    "${BUILD_DIR}/demo_main.o" \
    "${BUILD_DIR}/libSDL3_upstream.a" \
    -o "${BUILD_DIR}/sdl3_upstream_demo"

EXE_SIZE=$(stat -c%s "${BUILD_DIR}/sdl3_upstream_demo")
echo "[SUCCESS] Linked genuine upstream SDL3 executable: ${BUILD_DIR}/sdl3_upstream_demo (${EXE_SIZE} bytes)"

# 4. Package signed APB1 bundle
python3 -c "import sys; sys.path.insert(0, '${ROOT_DIR}/tools'); import apb1_format; \
manifest = apb1_format.make_manifest(app_id=b'org.arenaos.sdl3upstream', package_id=b'org.arenaos.sdl3upstream', \
display_name=b'SDL3 Upstream Demo', version=1, flags=25, requested=0, entry=b'bin/sdl3_upstream', icon=b'', width=320, height=240, associations=()); \
bundle = apb1_format.build_bundle([(1, b'bin/sdl3_upstream', open('${BUILD_DIR}/sdl3_upstream_demo', 'rb').read())], manifest=manifest); \
open('${BUILD_DIR}/SDL3Upstream.apb1', 'wb').write(bundle)"

BUNDLE_SIZE=$(stat -c%s "${BUILD_DIR}/SDL3Upstream.apb1")
echo "[SUCCESS] Packaged signed APB1 bundle: ${BUILD_DIR}/SDL3Upstream.apb1 (${BUNDLE_SIZE} bytes)"

# 5. Generate Symbol and Provenance Report
REPORT="${BUILD_DIR}/provenance_report.txt"
cat << EOF > "${REPORT}"
================================================================================
Milestone G2.1 Genuine Upstream SDL3 Linker Symbol & Provenance Report
================================================================================
Date: $(date -u +"%Y-%m-%d %H:%M:%SZ")
Upstream Version: SDL 3.2.0 official release (commit 535d80badefc83c5c527ec5748f2a20d6a9310fe)
Source Archive: experimental/upstream-sdl3/SDL-release-3.2.0
Static Library: ${BUILD_DIR}/libSDL3_upstream.a ($(stat -c%s "${BUILD_DIR}/libSDL3_upstream.a") bytes, 81 modules)
Binary Executable: ${BUILD_DIR}/sdl3_upstream_demo (${EXE_SIZE} bytes)
Signed Bundle: ${BUILD_DIR}/SDL3Upstream.apb1 (${BUNDLE_SIZE} bytes)

1. ELF Program Headers (Kernel PT_LOAD Segments):
$(readelf -l "${BUILD_DIR}/sdl3_upstream_demo")

2. Section Allocation Breakdown:
$(size -A -d "${BUILD_DIR}/sdl3_upstream_demo")

3. Key Upstream Function Provenance in Final Binary:
$(nm "${BUILD_DIR}/sdl3_upstream_demo" | grep -E "(PRIVATE_bootstrap|SDL_Init|SDL_CreateWindow|SDL_GetWindowSurface|SDL_UpdateWindowSurface|SDL_PollEvent|SDL_Quit)")

4. Disassembly Machine Code FP/SIMD Audit:
Total %xmm register occurrences: $(objdump -d "${BUILD_DIR}/sdl3_upstream_demo" | grep -cE "%xmm" || true)
Total x87 instruction occurrences: $(objdump -d "${BUILD_DIR}/sdl3_upstream_demo" | grep -cE "\s(f(ld|st|ild|ist|add|sub|mul|div|com|ucom|xch|cmov|ninit|nstcw|ldcw|nstsw|wait|sqrt|abs|chs|nop)[a-z0-9]*)\s" || true)

5. SHA-256 Hashes:
$(sha256sum "${BUILD_DIR}/libSDL3_upstream.a")
$(sha256sum "${BUILD_DIR}/sdl3_upstream_demo")
$(sha256sum "${BUILD_DIR}/SDL3Upstream.apb1")
================================================================================
EOF

cat "${REPORT}"
