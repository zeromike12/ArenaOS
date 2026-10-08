#include "SDL_internal.h"
#include "video/SDL_sysvideo.h"

static float s_mouse_x = 0.0f;
static float s_mouse_y = 0.0f;
static SDL_MouseButtonFlags s_mouse_buttons = 0;

void SDL_SendMouseMotion(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, bool relative, float x, float y) {
    (void)relative;
    SDL_Event event;
    SDL_zero(event);

    float rel_x = x - s_mouse_x;
    float rel_y = y - s_mouse_y;
    s_mouse_x = x;
    s_mouse_y = y;

    event.type = SDL_EVENT_MOUSE_MOTION;
    event.motion.timestamp = timestamp ? timestamp : SDL_GetTicksNS();
    event.motion.windowID = window ? window->id : 0;
    event.motion.which = mouseID;
    event.motion.state = s_mouse_buttons;
    event.motion.x = x;
    event.motion.y = y;
    event.motion.xrel = rel_x;
    event.motion.yrel = rel_y;

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
    event.button.x = s_mouse_x;
    event.button.y = s_mouse_y;

    SDL_PushEvent(&event);
}

void SDL_SendMouseWheel(Uint64 timestamp, SDL_Window *window, SDL_MouseID mouseID, float x, float y, SDL_MouseWheelDirection direction) {
    SDL_Event event;
    SDL_zero(event);

    event.type = SDL_EVENT_MOUSE_WHEEL;
    event.wheel.timestamp = timestamp ? timestamp : SDL_GetTicksNS();
    event.wheel.windowID = window ? window->id : 0;
    event.wheel.which = mouseID;
    event.wheel.x = x;
    event.wheel.y = y;
    event.wheel.direction = direction;
    event.wheel.mouse_x = s_mouse_x;
    event.wheel.mouse_y = s_mouse_y;

    SDL_PushEvent(&event);
}

SDL_MouseButtonFlags SDLCALL SDL_GetMouseState(float *x, float *y) {
    if (x) *x = s_mouse_x;
    if (y) *y = s_mouse_y;
    return s_mouse_buttons;
}
