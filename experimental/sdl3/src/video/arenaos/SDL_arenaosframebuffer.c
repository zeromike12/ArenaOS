#include "../../platform/arenaos/arenaos_syscalls.h"
#include "SDL_arenaosvideo.h"

bool ARENAOS_CreateWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window, SDL_PixelFormat *format, void **pixels, int *pitch) {
    arenaos_debug_write("[sdl3-app] -> ARENAOS_CreateWindowFramebuffer (upstream)\n", 56);
    (void)_this;
    if (!window || !window->internal) {
        return false;
    }

    struct SDL_WindowData *wdata = (struct SDL_WindowData *)window->internal;

    if (wdata->handle == 0) {
        const char *title = window->title ? window->title : "SDL3 App";
        if (!adsk_connect((uint16_t)window->w, (uint16_t)window->h, title)) {
            return false;
        }

        wdata->handle = adsk_get_handle();
        wdata->pixels = adsk_get_pixels();
        wdata->width = window->w;
        wdata->height = window->h;
        wdata->pitch = window->w * 4;
    }

    *format = SDL_PIXELFORMAT_XRGB8888;
    *pixels = wdata->pixels;
    *pitch = wdata->pitch;

    return true;
}

bool ARENAOS_UpdateWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window, const SDL_Rect *rects, int numrects) {
    (void)_this;
    if (!window || !window->internal) {
        return false;
    }

    struct SDL_WindowData *wdata = (struct SDL_WindowData *)window->internal;
    if (wdata->handle == 0) {
        return false;
    }

    if (!rects || numrects <= 0) {
        return adsk_damage_full();
    }

    if (numrects > 5) {
        /* ADSK-v1 wire format limits damage rects to 5.
         * Fall back to full-window damage to prevent dropping regions. */
        return adsk_damage_full();
    }

    uint16_t r[5][4];
    for (int i = 0; i < numrects; i++) {
        if (rects[i].w <= 0 || rects[i].h <= 0) {
            return adsk_damage_full();
        }

        int x1 = rects[i].x;
        int y1 = rects[i].y;
        int x2 = x1 + rects[i].w;
        int y2 = y1 + rects[i].h;

        /* Clip to window bounds */
        if (x1 < 0) x1 = 0;
        if (y1 < 0) y1 = 0;
        if (x2 > window->w) x2 = window->w;
        if (y2 > window->h) y2 = window->h;

        if (x2 <= x1 || y2 <= y1) {
            /* Empty or outside visible window */
            return adsk_damage_full();
        }

        r[i][0] = (uint16_t)x1;
        r[i][1] = (uint16_t)y1;
        r[i][2] = (uint16_t)(x2 - x1);
        r[i][3] = (uint16_t)(y2 - y1);
    }
    return adsk_damage_rects(numrects, r);
}

void ARENAOS_DestroyWindowFramebuffer(SDL_VideoDevice *_this, SDL_Window *window) {
    (void)_this;
    if (window && window->internal) {
        struct SDL_WindowData *wdata = (struct SDL_WindowData *)window->internal;
        if (wdata->handle != 0) {
            adsk_disconnect();
            wdata->handle = 0;
            wdata->pixels = NULL;
        }
    }
}
