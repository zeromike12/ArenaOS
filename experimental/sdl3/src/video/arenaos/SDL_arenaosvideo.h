#ifndef SDL_arenaosvideo_h_
#define SDL_arenaosvideo_h_

#include "../../SDL_internal.h"
#include "../SDL_sysvideo.h"
#include "../../platform/arenaos/arenaos_client.h"

struct SDL_VideoData {
    bool initialized;
    SDL_Window *primary_window;
};

struct SDL_WindowData {
    uint64_t handle;
    void *pixels;
    int width;
    int height;
    int pitch;
};

extern VideoBootStrap ARENAOS_bootstrap;

SDL_VideoDevice *ARENAOS_CreateDevice(void);
bool ARENAOS_VideoInit(SDL_VideoDevice *_this);
void ARENAOS_VideoQuit(SDL_VideoDevice *_this);
bool ARENAOS_CreateSDLWindow(SDL_VideoDevice *_this, SDL_Window *window, SDL_PropertiesID props);
void ARENAOS_DestroyWindow(SDL_VideoDevice *_this, SDL_Window *window);
bool ARENAOS_SetWindowTitle(SDL_VideoDevice *_this, SDL_Window *window);

bool ARENAOS_CreateWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window, SDL_PixelFormat *format, void **pixels, int *pitch);
bool ARENAOS_UpdateWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window, const SDL_Rect *rects, int numrects);
void ARENAOS_DestroyWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window);

void ARENAOS_PumpEvents(SDL_VideoDevice *_this);
void ARENAOS_DeleteDevice(SDL_VideoDevice *_this);

#endif /* SDL_arenaosvideo_h_ */
