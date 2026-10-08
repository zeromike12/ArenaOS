#!/usr/bin/env bash
# Build script for authentic upstream SDL 3.2.0 integration on ArenaOS
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
UPSTREAM_DIR="${SCRIPT_DIR}/SDL-release-3.2.0"
BUILD_DIR="${SCRIPT_DIR}/build"

echo "=== Building Genuine Upstream SDL 3.2.0 for ArenaOS ==="
mkdir -p "${BUILD_DIR}"

FLAGS="-I${UPSTREAM_DIR}/include -I${UPSTREAM_DIR}/src -I${UPSTREAM_DIR}/include/build_config \
-Os -ffreestanding -fno-builtin -nostdlib -fno-pie -no-pie \
-include ${UPSTREAM_DIR}/include/build_config/SDL_build_config_minimal.h \
-DSDL_STATIC_LIB=1 \
-DSDL_PLATFORM_PRIVATE=1 \
-DSDL_PLATFORM_PRIVATE_NAME=\"ArenaOS\" \
-DSDL_VIDEO_DRIVER_PRIVATE=1 \
-DSDL_AUDIO_DISABLED=1 \
-DSDL_CAMERA_DISABLED=1 \
-DSDL_GPU_DISABLED=1 \
-DSDL_RENDER_DISABLED=1 \
-DSDL_JOYSTICK_DISABLED=1 \
-DSDL_HAPTIC_DISABLED=1 \
-DSDL_HIDAPI_DISABLED=1 \
-DSDL_POWER_DISABLED=1 \
-DSDL_SENSOR_DISABLED=1 \
-DSDL_DIALOG_DISABLED=1 \
-DSDL_THREADS_DISABLED=1"

SRCS="
src/SDL.c
src/SDL_error.c
src/SDL_guid.c
src/SDL_hashtable.c
src/SDL_hints.c
src/SDL_log.c
src/SDL_properties.c
src/SDL_utils.c
src/SDL_assert.c
src/atomic/SDL_atomic.c
src/atomic/SDL_spinlock.c
src/cpuinfo/SDL_cpuinfo.c
src/events/SDL_events.c
src/events/SDL_keyboard.c
src/events/SDL_mouse.c
src/events/SDL_windowevents.c
src/events/SDL_quit.c
src/events/SDL_keymap.c
src/filesystem/dummy/SDL_sysfilesystem.c
src/loadso/dummy/SDL_sysloadso.c
src/process/dummy/SDL_dummyprocess.c
src/tray/SDL_tray_utils.c
src/stdlib/SDL_getenv.c
src/stdlib/SDL_iconv.c
src/stdlib/SDL_malloc.c
src/stdlib/SDL_memcpy.c
src/stdlib/SDL_memmove.c
src/stdlib/SDL_memset.c
src/stdlib/SDL_qsort.c
src/stdlib/SDL_stdlib.c
src/stdlib/SDL_string.c
src/stdlib/SDL_strtokr.c
src/stdlib/SDL_murmur3.c
src/thread/generic/SDL_syscond.c
src/thread/generic/SDL_sysmutex.c
src/thread/generic/SDL_sysrwlock.c
src/thread/generic/SDL_syssem.c
src/thread/generic/SDL_systls.c
src/thread/generic/SDL_systhread.c
src/thread/SDL_thread.c
src/timer/SDL_timer.c
src/video/SDL_video.c
src/video/SDL_surface.c
src/video/SDL_pixels.c
src/video/SDL_rect.c
src/video/SDL_blit.c
src/video/SDL_blit_0.c
src/video/SDL_blit_1.c
src/video/SDL_blit_A.c
src/video/SDL_blit_N.c
src/video/SDL_blit_slow.c
src/video/SDL_bmp.c
src/video/SDL_stretch.c
"

count=0
for s in $SRCS; do
    obj="${BUILD_DIR}/$(basename "${s%.c}.o")"
    gcc $FLAGS -c "${UPSTREAM_DIR}/${s}" -o "${obj}"
    count=$((count + 1))
done

ar rcs "${BUILD_DIR}/libSDL3_upstream.a" "${BUILD_DIR}"/*.o
echo "[SUCCESS] Compiled ${count} upstream source modules into ${BUILD_DIR}/libSDL3_upstream.a ($(stat -c%s "${BUILD_DIR}/libSDL3_upstream.a") bytes)"
