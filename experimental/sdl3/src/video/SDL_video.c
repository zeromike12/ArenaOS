#include "platform/arenaos/arenaos_syscalls.h"
#include "SDL_sysvideo.h"
#include "arenaos/SDL_arenaosvideo.h"

static SDL_VideoDevice *s_video_device = NULL;
static SDL_WindowID s_next_window_id = 0;
static SDL_DisplayID s_next_display_id = 0;

static VideoBootStrap *s_bootstrap[] = {
    &ARENAOS_bootstrap,
    NULL
};

SDL_VideoDevice *SDL_GetVideoDevice(void) {
    return s_video_device;
}

SDL_DisplayID SDL_AddBasicVideoDisplay(const SDL_DisplayMode *desktop_mode) {
    if (!s_video_device) return 0;

    SDL_VideoDisplay *display = (SDL_VideoDisplay *)SDL_calloc(1, sizeof(SDL_VideoDisplay));
    if (!display) return 0;

    display->id = ++s_next_display_id;
    display->name = "ArenaOS Default Display";
    if (desktop_mode) {
        display->desktop_mode = *desktop_mode;
        display->current_mode = *desktop_mode;
    }

    s_video_device->num_displays = 1;
    s_video_device->displays = (SDL_VideoDisplay **)SDL_calloc(1, sizeof(SDL_VideoDisplay *));
    s_video_device->displays[0] = display;

    return display->id;
}

bool SDLCALL SDL_VideoInit(const char *driver_name) {
    arenaos_debug_write("[sdl3-app] -> SDL_VideoInit\n", 27);
    (void)driver_name;
    if (s_video_device) {
        return true; /* Already initialized */
    }

    for (int i = 0; s_bootstrap[i]; i++) {
        SDL_VideoDevice *device = s_bootstrap[i]->create();
        if (device) {
            s_video_device = device;
            if (device->VideoInit && !device->VideoInit(device)) {
                device->free(device);
                s_video_device = NULL;
                continue;
            }
            return true;
        }
    }

    SDL_SetError("No available video device");
    return false;
}

void SDLCALL SDL_VideoQuit(void) {
    if (s_video_device) {
        /* Destroy all windows */
        while (s_video_device->windows) {
            SDL_DestroyWindow(s_video_device->windows);
        }

        if (s_video_device->VideoQuit) {
            s_video_device->VideoQuit(s_video_device);
        }

        if (s_video_device->displays) {
            for (int i = 0; i < s_video_device->num_displays; i++) {
                if (s_video_device->displays[i]) {
                    SDL_free(s_video_device->displays[i]);
                }
            }
            SDL_free(s_video_device->displays);
            s_video_device->displays = NULL;
            s_video_device->num_displays = 0;
        }

        if (s_video_device->free) {
            s_video_device->free(s_video_device);
        }
        s_video_device = NULL;
    }
}

SDL_Window * SDLCALL SDL_CreateWindow(const char *title, int w, int h, SDL_WindowFlags flags) {
    arenaos_debug_write("[sdl3-app] -> SDL_CreateWindow\n", 30);
    if (!s_video_device) {
        if (!SDL_VideoInit(NULL)) {
            return NULL;
        }
    }

    if (w <= 0 || h <= 0) {
        SDL_SetError("Invalid window dimensions (%dx%d)", w, h);
        return NULL;
    }

    SDL_Window *window = (SDL_Window *)SDL_calloc(1, sizeof(SDL_Window));
    if (!window) {
        SDL_OutOfMemory();
        return NULL;
    }

    window->id = ++s_next_window_id;
    window->title = title ? SDL_strdup(title) : SDL_strdup("SDL3 Window");
    window->w = w;
    window->h = h;
    window->flags = flags;
    window->surface = NULL;
    window->surface_valid = false;

    if (s_video_device->CreateSDLWindow && !s_video_device->CreateSDLWindow(s_video_device, window, 0)) {
        SDL_free(window->title);
        SDL_free(window);
        SDL_SetError("Failed to create platform window");
        return NULL;
    }

    /* Link into window list */
    window->next = s_video_device->windows;
    s_video_device->windows = window;

    return window;
}

void SDLCALL SDL_DestroyWindow(SDL_Window *window) {
    if (!window || !s_video_device) return;

    SDL_DestroyWindowSurface(window);

    if (s_video_device->DestroyWindow) {
        s_video_device->DestroyWindow(s_video_device, window);
    }

    /* Unlink from device list */
    SDL_Window **curr = &s_video_device->windows;
    while (*curr) {
        if (*curr == window) {
            *curr = window->next;
            break;
        }
        curr = &(*curr)->next;
    }

    if (window->title) {
        SDL_free(window->title);
    }
    SDL_free(window);
}

SDL_Surface * SDLCALL SDL_GetWindowSurface(SDL_Window *window) {
    arenaos_debug_write("[sdl3-app] -> SDL_GetWindowSurface\n", 34);
    if (!window || !s_video_device) {
        SDL_SetError("Invalid window for surface query");
        return NULL;
    }

    if (window->surface && window->surface_valid) {
        return window->surface;
    }

    SDL_PixelFormat format;
    void *pixels = NULL;
    int pitch = 0;

    if (!s_video_device->CreateWindowFramebuffer ||
        !s_video_device->CreateWindowFramebuffer(s_video_device, window, &format, &pixels, &pitch)) {
        SDL_SetError("Failed to create window framebuffer");
        return NULL;
    }

    window->surface = SDL_CreateSurfaceFrom(window->w, window->h, format, pixels, pitch);
    if (!window->surface) {
        return NULL;
    }

    window->surface_valid = true;
    return window->surface;
}

bool SDLCALL SDL_UpdateWindowSurface(SDL_Window *window) {
    return SDL_UpdateWindowSurfaceRects(window, NULL, 0);
}

bool SDLCALL SDL_UpdateWindowSurfaceRects(SDL_Window *window, const SDL_Rect *rects, int numrects) {
    if (!window || !s_video_device) {
        return SDL_SetError("Invalid window for surface update");
    }

    if (s_video_device->UpdateWindowFramebuffer) {
        return s_video_device->UpdateWindowFramebuffer(s_video_device, window, rects, numrects);
    }

    return true;
}

bool SDLCALL SDL_DestroyWindowSurface(SDL_Window *window) {
    if (!window || !s_video_device) return false;

    if (window->surface) {
        SDL_DestroySurface(window->surface);
        window->surface = NULL;
        window->surface_valid = false;
    }

    if (s_video_device->DestroyWindowFramebuffer) {
        s_video_device->DestroyWindowFramebuffer(s_video_device, window);
    }

    return true;
}

bool SDLCALL SDL_WindowHasSurface(SDL_Window *window) {
    return (window && window->surface && window->surface_valid);
}

bool SDLCALL SDL_SetWindowTitle(SDL_Window *window, const char *title) {
    if (!window) return false;
    if (window->title) {
        SDL_free(window->title);
    }
    window->title = title ? SDL_strdup(title) : NULL;

    if (s_video_device && s_video_device->SetWindowTitle) {
        return s_video_device->SetWindowTitle(s_video_device, window);
    }
    return true;
}

const char * SDLCALL SDL_GetWindowTitle(SDL_Window *window) {
    return window ? window->title : NULL;
}

bool SDLCALL SDL_GetWindowSize(SDL_Window *window, int *w, int *h) {
    if (!window) return false;
    if (w) *w = window->w;
    if (h) *h = window->h;
    return true;
}

SDL_WindowID SDLCALL SDL_GetWindowID(SDL_Window *window) {
    return window ? window->id : 0;
}

