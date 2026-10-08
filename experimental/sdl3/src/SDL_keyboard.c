#include "SDL_internal.h"
#include "video/SDL_sysvideo.h"

void SDL_SendKeyboardKey(Uint64 timestamp, SDL_KeyboardID keyboardID, int rawcode, SDL_Scancode scancode, bool down) {
    SDL_Event event;
    SDL_zero(event);

    event.type = down ? SDL_EVENT_KEY_DOWN : SDL_EVENT_KEY_UP;
    event.key.timestamp = timestamp ? timestamp : SDL_GetTicksNS();
    event.key.which = keyboardID;
    event.key.scancode = scancode;
    event.key.key = (SDL_Keycode)rawcode;
    event.key.raw = (Uint16)rawcode;
    event.key.down = down;
    event.key.repeat = false;

    SDL_VideoDevice *device = SDL_GetVideoDevice();
    if (device && device->windows) {
        event.key.windowID = device->windows->id;
    }

    SDL_PushEvent(&event);
}
