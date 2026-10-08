#include "platform/arenaos/arenaos_syscalls.h"
#include "SDL_internal.h"
#include <SDL3/SDL_revision.h>
#include "video/SDL_sysvideo.h"

static Uint32 s_initialized_flags = 0;
static bool s_main_is_ready = true;

extern bool SDL_InitEvents(void);
extern void SDL_QuitEvents(void);
extern bool SDL_VideoInit(const char *driver_name);
extern void SDL_VideoQuit(void);

void SDLCALL SDL_SetMainReady(void) {
    s_main_is_ready = true;
}

bool SDLCALL SDL_IsMainThread(void) {
    return true;
}

const char * SDLCALL SDL_GetRevision(void) {
    return SDL_REVISION;
}

bool SDLCALL SDL_InitSubSystem(SDL_InitFlags flags) {
    arenaos_debug_write("[sdl3-app] -> SDL_InitSubSystem\n", 31);
    if (flags & SDL_INIT_VIDEO) {
        if (!(s_initialized_flags & SDL_INIT_EVENTS)) {
            if (!SDL_InitEvents()) {
                return false;
            }
            s_initialized_flags |= SDL_INIT_EVENTS;
        }

        if (!(s_initialized_flags & SDL_INIT_VIDEO)) {
            if (!SDL_VideoInit(NULL)) {
                return false;
            }
            s_initialized_flags |= SDL_INIT_VIDEO;
        }
    }

    if (flags & SDL_INIT_EVENTS) {
        if (!(s_initialized_flags & SDL_INIT_EVENTS)) {
            if (!SDL_InitEvents()) {
                return false;
            }
            s_initialized_flags |= SDL_INIT_EVENTS;
        }
    }

    return true;
}

void SDLCALL SDL_QuitSubSystem(SDL_InitFlags flags) {
    if ((flags & SDL_INIT_VIDEO) && (s_initialized_flags & SDL_INIT_VIDEO)) {
        SDL_VideoQuit();
        s_initialized_flags &= ~SDL_INIT_VIDEO;
    }

    if ((flags & SDL_INIT_EVENTS) && (s_initialized_flags & SDL_INIT_EVENTS)) {
        SDL_QuitEvents();
        s_initialized_flags &= ~SDL_INIT_EVENTS;
    }
}

SDL_InitFlags SDLCALL SDL_WasInit(SDL_InitFlags flags) {
    if (flags == 0) {
        return s_initialized_flags;
    }
    return (s_initialized_flags & flags);
}

bool SDLCALL SDL_Init(SDL_InitFlags flags) {
    arenaos_debug_write("[sdl3-app] -> SDL_Init\n", 22);
    return SDL_InitSubSystem(flags);
}

void SDLCALL SDL_Quit(void) {
    SDL_QuitSubSystem(s_initialized_flags);
}
