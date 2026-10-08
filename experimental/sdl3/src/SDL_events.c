#include "SDL_internal.h"
#include "video/SDL_sysvideo.h"

#define EVENT_QUEUE_SIZE 64

static SDL_Event s_queue[EVENT_QUEUE_SIZE];
static int s_head = 0;
static int s_tail = 0;

bool SDL_InitEvents(void) {
    s_head = 0;
    s_tail = 0;
    return true;
}

void SDL_QuitEvents(void) {
    s_head = 0;
    s_tail = 0;
}

bool SDLCALL SDL_PushEvent(SDL_Event *event) {
    if (!event) return false;

    int next_tail = (s_tail + 1) % EVENT_QUEUE_SIZE;
    if (next_tail == s_head) {
        /* Queue full: drop event */
        return false;
    }

    s_queue[s_tail] = *event;
    s_tail = next_tail;
    return true;
}

void SDLCALL SDL_PumpEvents(void) {
    SDL_VideoDevice *device = SDL_GetVideoDevice();
    if (device && device->PumpEvents) {
        device->PumpEvents(device);
    }
}

bool SDLCALL SDL_PollEvent(SDL_Event *event) {
    SDL_PumpEvents();

    if (s_head == s_tail) {
        return false;
    }

    if (event) {
        *event = s_queue[s_head];
    }
    s_head = (s_head + 1) % EVENT_QUEUE_SIZE;
    return true;
}

bool SDLCALL SDL_WaitEvent(SDL_Event *event) {
    while (1) {
        if (SDL_PollEvent(event)) {
            return true;
        }
        SDL_Delay(5);
    }
}

bool SDLCALL SDL_WaitEventTimeout(SDL_Event *event, Sint32 timeoutMS) {
    Uint64 start = SDL_GetTicks();
    while (1) {
        if (SDL_PollEvent(event)) {
            return true;
        }
        if (timeoutMS >= 0 && (Sint32)(SDL_GetTicks() - start) >= timeoutMS) {
            return false;
        }
        SDL_Delay(2);
    }
}

void SDLCALL SDL_FlushEvents(Uint32 minType, Uint32 maxType) {
    int cur = s_head;
    int write_pos = s_head;

    while (cur != s_tail) {
        Uint32 type = s_queue[cur].type;
        if (type < minType || type > maxType) {
            if (write_pos != cur) {
                s_queue[write_pos] = s_queue[cur];
            }
            write_pos = (write_pos + 1) % EVENT_QUEUE_SIZE;
        }
        cur = (cur + 1) % EVENT_QUEUE_SIZE;
    }
    s_tail = write_pos;
}

bool SDLCALL SDL_HasEvent(Uint32 type) {
    int cur = s_head;
    while (cur != s_tail) {
        if (s_queue[cur].type == type) {
            return true;
        }
        cur = (cur + 1) % EVENT_QUEUE_SIZE;
    }
    return false;
}
