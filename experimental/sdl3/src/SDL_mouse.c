#include "SDL_internal.h"
#include "video/SDL_sysvideo.h"
#include "arenaos_ieee754.h"

static int s_mouse_x = 0;
static int s_mouse_y = 0;
static SDL_MouseButtonFlags s_mouse_buttons = 0;

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
    ux.u = arenaos_int32_to_float_bits(x);
    uy.u = arenaos_int32_to_float_bits(y);
    urx.u = arenaos_int32_to_float_bits(rel_x);
    ury.u = arenaos_int32_to_float_bits(rel_y);

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

    union { float f; uint32_t u; } ux = { .u = arenaos_int32_to_float_bits(s_mouse_x) };
    union { float f; uint32_t u; } uy = { .u = arenaos_int32_to_float_bits(s_mouse_y) };
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

    union { float f; uint32_t u; } ux = { .u = arenaos_int32_to_float_bits(x) };
    union { float f; uint32_t u; } uy = { .u = arenaos_int32_to_float_bits(y) };
    union { float f; uint32_t u; } umx = { .u = arenaos_int32_to_float_bits(s_mouse_x) };
    union { float f; uint32_t u; } umy = { .u = arenaos_int32_to_float_bits(s_mouse_y) };

    event.wheel.x = ux.f;
    event.wheel.y = uy.f;
    event.wheel.mouse_x = umx.f;
    event.wheel.mouse_y = umy.f;

    SDL_PushEvent(&event);
}

SDL_MouseButtonFlags SDLCALL SDL_GetMouseState(float *x, float *y) {
    if (x) {
        union { float f; uint32_t u; } ux = { .u = arenaos_int32_to_float_bits(s_mouse_x) };
        *x = ux.f;
    }
    if (y) {
        union { float f; uint32_t u; } uy = { .u = arenaos_int32_to_float_bits(s_mouse_y) };
        *y = uy.f;
    }
    return s_mouse_buttons;
}
