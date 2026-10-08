#ifndef SDL_build_config_h_
#define SDL_build_config_h_

#define SDL_PLATFORM_ARENAOS 1

/* Subsystems disabled for freestanding microkernel target */
#define SDL_AUDIO_DISABLED 1
#define SDL_JOYSTICK_DISABLED 1
#define SDL_HAPTIC_DISABLED 1
#define SDL_HIDAPI_DISABLED 1
#define SDL_CAMERA_DISABLED 1
#define SDL_GPU_DISABLED 1
#define SDL_SENSOR_DISABLED 1
#define SDL_DIALOG_DISABLED 1
#define SDL_POWER_DISABLED 1
#define SDL_TRAY_DISABLED 1
#define SDL_PROCESS_DISABLED 1
#define SDL_FILESYSTEM_DISABLED 1
#define SDL_STORAGE_DISABLED 1
#define SDL_THREADS_DISABLED 1

/* Subsystems enabled */
#define SDL_VIDEO_DRIVER_ARENAOS 1
#define SDL_EVENTS_DISABLED 0
#define SDL_TIMERS_DISABLED 0
#define SDL_LEAN_AND_MEAN 1
#define SDL_STATIC_LIB 1

/* Types and attributes */
#define SIZEOF_VOIDP 8

#ifndef __cplusplus
#define HAVE_GCC_ATOMICS 1
#endif

#endif /* SDL_build_config_h_ */
