#include "SDL_internal.h"

extern Uint64 SDL_SYS_GetTicks(void);
extern Uint64 SDL_SYS_GetTicksNS(void);
extern void SDL_SYS_DelayNS(Uint64 ns);

Uint64 SDLCALL SDL_GetTicks(void) {
    return SDL_SYS_GetTicks();
}

Uint64 SDLCALL SDL_GetTicksNS(void) {
    return SDL_SYS_GetTicksNS();
}

Uint64 SDLCALL SDL_GetPerformanceCounter(void) {
    return SDL_SYS_GetTicksNS();
}

Uint64 SDLCALL SDL_GetPerformanceFrequency(void) {
    return 1000000000ULL; /* 1 GHz / nanosecond resolution */
}

void SDLCALL SDL_Delay(Uint32 ms) {
    SDL_SYS_DelayNS((Uint64)ms * 1000000ULL);
}

void SDLCALL SDL_DelayNS(Uint64 ns) {
    SDL_SYS_DelayNS(ns);
}

void SDLCALL SDL_DelayPrecise(Uint64 ns) {
    SDL_SYS_DelayNS(ns);
}
