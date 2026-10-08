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

    if (rects && numrects > 0) {
        int count = (numrects > 5) ? 5 : numrects;
        uint16_t r[5][4];
        for (int i = 0; i < count; i++) {
            int x = (rects[i].x < 0) ? 0 : rects[i].x;
            int y = (rects[i].y < 0) ? 0 : rects[i].y;
            int w = rects[i].w;
            int h = rects[i].h;
            if (x + w > window->w) w = window->w - x;
            if (y + h > window->h) h = window->h - y;
            if (w <= 0 || h <= 0) {
                w = 1;
                h = 1;
            }
            r[i][0] = (uint16_t)x;
            r[i][1] = (uint16_t)y;
            r[i][2] = (uint16_t)w;
            r[i][3] = (uint16_t)h;
        }
        return adsk_damage_rects(count, r);
    } else {
        return adsk_damage_full();
    }
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
