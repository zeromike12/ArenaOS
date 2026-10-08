#include "SDL_internal.h"
#include "video/SDL_sysvideo.h"

void SDL_SendWindowEvent(SDL_Window *window, SDL_EventType windowevent, int data1, int data2) {
    SDL_Event event;
    SDL_zero(event);

    event.type = windowevent;
    event.window.timestamp = SDL_GetTicksNS();
    event.window.windowID = window ? window->id : 0;
    event.window.data1 = (Sint32)data1;
    event.window.data2 = (Sint32)data2;

    SDL_PushEvent(&event);
}
