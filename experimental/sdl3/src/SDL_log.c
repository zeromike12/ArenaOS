#include "SDL_internal.h"
#include "platform/arenaos/arenaos_syscalls.h"
#include <stdarg.h>

void SDLCALL SDL_Log(SDL_PRINTF_FORMAT_STRING const char *fmt, ...) {
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    int len = SDL_vsnprintf(buf, sizeof(buf) - 2, fmt, ap);
    va_end(ap);

    if (len > 0) {
        if (buf[len - 1] != '\n') {
            buf[len++] = '\n';
            buf[len] = '\0';
        }
        arenaos_debug_write(buf, (size_t)len);
    }
}

void SDLCALL SDL_LogInfo(int category, SDL_PRINTF_FORMAT_STRING const char *fmt, ...) {
    (void)category;
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    int len = SDL_vsnprintf(buf, sizeof(buf) - 2, fmt, ap);
    va_end(ap);

    if (len > 0) {
        if (buf[len - 1] != '\n') {
            buf[len++] = '\n';
            buf[len] = '\0';
        }
        arenaos_debug_write(buf, (size_t)len);
    }
}
