#include "SDL_internal.h"
#include <stdarg.h>

static char s_error_message[512] = "";

bool SDLCALL SDL_SetErrorV(const char *fmt, va_list ap) {
    if (!fmt) return false;
    SDL_vsnprintf(s_error_message, sizeof(s_error_message), fmt, ap);
    return false;
}

bool SDLCALL SDL_SetError(const char *fmt, ...) {
    if (!fmt) return false;
    va_list ap;
    va_start(ap, fmt);
    SDL_SetErrorV(fmt, ap);
    va_end(ap);
    return false;
}

const char * SDLCALL SDL_GetError(void) {
    return s_error_message;
}

bool SDLCALL SDL_ClearError(void) {
    s_error_message[0] = '\0';
    return true;
}

bool SDLCALL SDL_OutOfMemory(void) {
    return SDL_SetError("Out of memory");
}
