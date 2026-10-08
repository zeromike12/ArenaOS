#include "../../SDL_internal.h"
#include "../../video/arenaos/SDL_arenaosvideo.h"
#include "../../platform/arenaos/arenaos_syscalls.h"

Uint64 SDL_GetPerformanceCounter(void) {
    return arenaos_clock_now();
}

Uint64 SDL_GetPerformanceFrequency(void) {
    return 1000000ULL; /* microsecond ticks */
}

void SDL_SYS_DelayNS(Uint64 ns) {
    uint32_t ms = (uint32_t)(ns / 1000000ULL);
    if (ms == 0 && ns > 0) {
        ms = 1;
    }
    adsk_sleep_ms(ms);
}
