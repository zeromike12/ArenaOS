/*
  Simple DirectMedia Layer - ArenaOS Build Configuration
  Milestone G2.1 Genuine Upstream SDL3 Integration
*/

#ifndef SDL_build_config_arenaos_h_
#define SDL_build_config_arenaos_h_

#define SDL_build_config_h_

#include <SDL3/SDL_platform_defines.h>

/* Neutralize host Linux definitions to prevent glibc/POSIX symbol leakage */
#undef SDL_PLATFORM_LINUX
#undef __linux__
#undef __linux
#undef linux

#define SDL_PLATFORM_ARENAOS 1
#define SDL_PLATFORM_PRIVATE 1
#define SDL_PLATFORM_PRIVATE_NAME "ArenaOS"
#define SDL_VIDEO_DRIVER_PRIVATE 1
#define SDL_STATIC_LIB 1

#define HAVE_STDARG_H 1
#define HAVE_STDDEF_H 1
#define HAVE_STDINT_H 1
#define HAVE_MALLOC 1

/* Enable lean and mean build to omit heavy SIMD/YUV codecs */
#define SDL_LEAN_AND_MEAN 1

/* Disable subsystems unneeded for basic software windowing */
#define SDL_AUDIO_DISABLED 1
#define SDL_CAMERA_DISABLED 1
#define SDL_GPU_DISABLED 1
#define SDL_RENDER_DISABLED 1
#define SDL_JOYSTICK_DISABLED 1
#define SDL_HAPTIC_DISABLED 1
#define SDL_HIDAPI_DISABLED 1
#define SDL_POWER_DISABLED 1
#define SDL_SENSOR_DISABLED 1
#define SDL_DIALOG_DISABLED 1
#define SDL_THREADS_DISABLED 1
#define SDL_ASYNCIO_DISABLED 1

/* Select dummy backend drivers for non-graphics services */
#define SDL_FILESYSTEM_DUMMY 1
#define SDL_FSOPS_DUMMY 1
#define SDL_LOADSO_DUMMY 1
#define SDL_PROCESS_DUMMY 1
#define SDL_TIMER_DUMMY 1

#endif /* SDL_build_config_arenaos_h_ */
