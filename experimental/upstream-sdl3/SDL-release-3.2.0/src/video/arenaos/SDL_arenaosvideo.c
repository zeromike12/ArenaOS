#include "SDL_arenaosvideo.h"

void ARENAOS_DeleteDevice(SDL_VideoDevice *device) {
    if (device) {
        if (device->internal) {
            SDL_free(device->internal);
            device->internal = NULL;
        }
        SDL_free(device);
    }
}

SDL_VideoDevice *ARENAOS_CreateDevice(void) {
    SDL_VideoDevice *device = (SDL_VideoDevice *)SDL_calloc(1, sizeof(SDL_VideoDevice));
    if (!device) {
        return NULL;
    }

    struct SDL_VideoData *data = (struct SDL_VideoData *)SDL_calloc(1, sizeof(struct SDL_VideoData));
    if (!data) {
        SDL_free(device);
        return NULL;
    }

    device->internal = (SDL_VideoData *)data;

    /* Setup official upstream SDL_VideoDevice vtable callbacks */
    device->name = "arenaos";
    device->VideoInit = ARENAOS_VideoInit;
    device->VideoQuit = ARENAOS_VideoQuit;
    device->CreateSDLWindow = ARENAOS_CreateSDLWindow;
    device->DestroyWindow = ARENAOS_DestroyWindow;
    device->SetWindowTitle = ARENAOS_SetWindowTitle;
    device->CreateWindowFramebuffer = ARENAOS_CreateWindowFramebuffer;
    device->UpdateWindowFramebuffer = ARENAOS_UpdateWindowFramebuffer;
    device->DestroyWindowFramebuffer = ARENAOS_DestroyWindowFramebuffer;
    device->PumpEvents = ARENAOS_PumpEvents;
    device->free = ARENAOS_DeleteDevice;

    return device;
}

/* Genuine upstream driver bootstrap structure */
VideoBootStrap PRIVATE_bootstrap = {
    "arenaos",
    "ArenaOS Desktop Display Driver",
    ARENAOS_CreateDevice,
    NULL
};

bool ARENAOS_VideoInit(SDL_VideoDevice *_this) {
    SDL_DisplayMode mode;
    SDL_zero(mode);
    mode.format = SDL_PIXELFORMAT_XRGB8888;
    mode.w = 800;
    mode.h = 600;
    mode.refresh_rate = 60.0f;

    if (SDL_AddBasicVideoDisplay(&mode) == 0) {
        SDL_SetError("ArenaOS: Failed to register default display mode");
        return false;
    }

    struct SDL_VideoData *vdata = (struct SDL_VideoData *)_this->internal;
    if (vdata) {
        vdata->initialized = true;
    }

    return true;
}

void ARENAOS_VideoQuit(SDL_VideoDevice *_this) {
    struct SDL_VideoData *vdata = (struct SDL_VideoData *)_this->internal;
    if (vdata) {
        vdata->initialized = false;
        vdata->primary_window = NULL;
    }
}

bool ARENAOS_CreateSDLWindow(SDL_VideoDevice *_this, SDL_Window *window, SDL_PropertiesID props) {
    (void)props;
    if (!_this || !window) {
        return false;
    }

    struct SDL_VideoData *vdata = (struct SDL_VideoData *)_this->internal;
    if (vdata && vdata->primary_window != NULL) {
        SDL_SetError("ArenaOS ADSK-v1 allows only one window per process");
        return false;
    }

    struct SDL_WindowData *wdata = (struct SDL_WindowData *)SDL_calloc(1, sizeof(struct SDL_WindowData));
    if (!wdata) {
        SDL_SetError("ArenaOS: Out of memory allocating SDL_WindowData");
        return false;
    }

    wdata->width = window->w;
    wdata->height = window->h;
    wdata->pitch = window->w * 4;
    window->internal = wdata;

    if (vdata) {
        vdata->primary_window = window;
    }

    return true;
}

void ARENAOS_DestroyWindow(SDL_VideoDevice *_this, SDL_Window *window) {
    if (_this && _this->internal) {
        struct SDL_VideoData *vdata = (struct SDL_VideoData *)_this->internal;
        if (vdata->primary_window == window) {
            vdata->primary_window = NULL;
        }
    }

    if (window && window->internal) {
        struct SDL_WindowData *wdata = (struct SDL_WindowData *)window->internal;
        if (wdata->handle != 0) {
            ARENAOS_DestroyWindowFramebuffer(_this, window);
        }
        SDL_free(wdata);
        window->internal = NULL;
    }
}

void ARENAOS_SetWindowTitle(SDL_VideoDevice *_this, SDL_Window *window) {
    (void)_this;
    if (!window || !window->internal) return;
    struct SDL_WindowData *wdata = (struct SDL_WindowData *)window->internal;
    if (wdata->handle != 0 && window->title) {
        /* Title update is handled on connect or through client title op */
    }
}

/* AsyncIO subsystem stub for freestanding environment */
void SDL_QuitAsyncIO(void) {}
