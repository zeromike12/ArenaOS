#include "SDL_internal.h"
#include "video/SDL_sysvideo.h"

static int s_mouse_x = 0;
static int s_mouse_y = 0;
static SDL_MouseButtonFlags s_mouse_buttons = 0;

/* Pure integer bit manipulation for IEEE-754 single precision float (0 FPU / 0 SSE) */
static inline uint32_t int_to_float_bits(int32_t val) {
    if (val == 0) return 0;
    uint32_t sign = 0;
    if (val < 0) {
        sign = 0x80000000U;
        val = -val;
    }
    int msb = 31 - __builtin_clz((uint32_t)val);
    uint32_t exp = (uint32_t)(msb + 127) << 23;
    uint32_t mantissa;
    if (msb >= 23) {
        mantissa = ((uint32_t)val >> (msb - 23)) & 0x7FFFFFU;
    } else {
        mantissa = ((uint32_t)val << (23 - msb)) & 0x7FFFFFU;
    }
    return sign | exp | mantissa;
}

void SDL_SendMouseMotion(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, bool relative, int x, int y) {
    (void)relative;
    SDL_Event event;
    SDL_zero(event);

    int rel_x = x - s_mouse_x;
    int rel_y = y - s_mouse_y;
    s_mouse_x = x;
    s_mouse_y = y;

    event.type = SDL_EVENT_MOUSE_MOTION;
    event.motion.timestamp = timestamp ? timestamp : SDL_GetTicksNS();
    event.motion.windowID = window ? window->id : 0;
    event.motion.which = mouseID;
    event.motion.state = s_mouse_buttons;

    union { float f; uint32_t u; } ux, uy, urx, ury;
    ux.u = int_to_float_bits(x);
    uy.u = int_to_float_bits(y);
    urx.u = int_to_float_bits(rel_x);
    ury.u = int_to_float_bits(rel_y);

    event.motion.x = ux.f;
    event.motion.y = uy.f;
    event.motion.xrel = urx.f;
    event.motion.yrel = ury.f;

    SDL_PushEvent(&event);
}

void SDL_SendMouseButton(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, Uint8 button, bool down) {
    SDL_Event event;
    SDL_zero(event);

    if (down) {
        s_mouse_buttons |= SDL_BUTTON_MASK(button);
    } else {
        s_mouse_buttons &= ~SDL_BUTTON_MASK(button);
    }

    event.type = down ? SDL_EVENT_MOUSE_BUTTON_DOWN : SDL_EVENT_MOUSE_BUTTON_UP;
    event.button.timestamp = timestamp ? timestamp : SDL_GetTicksNS();
    event.button.windowID = window ? window->id : 0;
    event.button.which = mouseID;
    event.button.button = button;
    event.button.down = down;
    event.button.clicks = 1;

    union { float f; uint32_t u; } ux = { .u = int_to_float_bits(s_mouse_x) };
    union { float f; uint32_t u; } uy = { .u = int_to_float_bits(s_mouse_y) };
    event.button.x = ux.f;
    event.button.y = uy.f;

    SDL_PushEvent(&event);
}

void SDL_SendMouseWheel(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, int x, int y, SDL_MouseWheelDirection direction) {
    SDL_Event event;
    SDL_zero(event);

    event.type = SDL_EVENT_MOUSE_WHEEL;
    event.wheel.timestamp = timestamp ? timestamp : SDL_GetTicksNS();
    event.wheel.windowID = window ? window->id : 0;
    event.wheel.which = mouseID;
    event.wheel.direction = direction;

    union { float f; uint32_t u; } ux = { .u = int_to_float_bits(x) };
    union { float f; uint32_t u; } uy = { .u = int_to_float_bits(y) };
    union { float f; uint32_t u; } umx = { .u = int_to_float_bits(s_mouse_x) };
    union { float f; uint32_t u; } umy = { .u = int_to_float_bits(s_mouse_y) };

    event.wheel.x = ux.f;
    event.wheel.y = uy.f;
    event.wheel.mouse_x = umx.f;
    event.wheel.mouse_y = umy.f;

    SDL_PushEvent(&event);
}

SDL_MouseButtonFlags SDLCALL SDL_GetMouseState(float *x, float *y) {
    if (x) {
        union { float f; uint32_t u; } ux = { .u = int_to_float_bits(s_mouse_x) };
        *x = ux.f;
    }
    if (y) {
        union { float f; uint32_t u; } uy = { .u = int_to_float_bits(s_mouse_y) };
        *y = uy.f;
    }
    return s_mouse_buttons;
}
