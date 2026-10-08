#include "SDL_arenaosvideo.h"
#include "../../platform/arenaos/arenaos_syscalls.h"

static uint64_t s_ticks_base_us = 0;

void SDL_SYS_InitTicks(void) {
    s_ticks_base_us = arenaos_clock_now();
}

Uint64 SDL_SYS_GetTicks(void) {
    if (s_ticks_base_us == 0) {
        SDL_SYS_InitTicks();
    }
    uint64_t now_us = arenaos_clock_now();
    return (now_us >= s_ticks_base_us) ? ((now_us - s_ticks_base_us) / 1000ULL) : 0;
}

Uint64 SDL_SYS_GetTicksNS(void) {
    if (s_ticks_base_us == 0) {
        SDL_SYS_InitTicks();
    }
    uint64_t now_us = arenaos_clock_now();
    return (now_us >= s_ticks_base_us) ? ((now_us - s_ticks_base_us) * 1000ULL) : 0;
}

void SDL_SYS_DelayNS(Uint64 ns) {
    uint32_t ms = (uint32_t)(ns / 1000000ULL);
    if (ms == 0 && ns > 0) {
        ms = 1;
    }
    adsk_sleep_ms(ms);
}
