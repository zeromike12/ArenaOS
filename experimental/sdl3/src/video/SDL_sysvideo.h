#ifndef SDL_sysvideo_h_
#define SDL_sysvideo_h_

#include "../SDL_internal.h"

typedef struct SDL_Window SDL_Window;
typedef struct SDL_VideoDisplay SDL_VideoDisplay;
typedef struct SDL_VideoDevice SDL_VideoDevice;

struct SDL_Window {
    const void *reserved;
    SDL_WindowID id;
    char *title;
    int x;
    int y;
    int w;
    int h;
    int min_w;
    int min_h;
    int max_w;
    int max_h;
    SDL_WindowFlags flags;
    SDL_Surface *surface;
    bool surface_valid;
    void *internal;
    struct SDL_Window *next;
};

struct SDL_VideoDisplay {
    SDL_DisplayID id;
    char *name;
    int max_fullscreen_modes;
    int num_fullscreen_modes;
    SDL_DisplayMode *fullscreen_modes;
    SDL_DisplayMode desktop_mode;
    SDL_DisplayMode current_mode;
    SDL_DisplayOrientation orientation;
    SDL_Window *fullscreen_window;
    void *internal;
};

struct SDL_VideoDevice {
    /* Function pointers */
    bool (*VideoInit)(SDL_VideoDevice *_this);
    void (*VideoQuit)(SDL_VideoDevice *_this);
    void (*ResetTouch)(SDL_VideoDevice *_this);

    bool (*CreateSDLWindow)(SDL_VideoDevice *_this, SDL_Window *window, SDL_PropertiesID props);
    void (*DestroyWindow)(SDL_VideoDevice *_this, SDL_Window *window);
    bool (*SetWindowTitle)(SDL_VideoDevice *_this, SDL_Window *window);

    bool (*CreateWindowFramebuffer)(SDL_VideoDevice *_this, SDL_Window *window, SDL_PixelFormat *format, void **pixels, int *pitch);
    bool (*UpdateWindowFramebuffer)(SDL_VideoDevice *_this, SDL_Window *window, const SDL_Rect *rects, int numrects);
    void (*DestroyWindowFramebuffer)(SDL_VideoDevice *_this, SDL_Window *window);

    void (*PumpEvents)(SDL_VideoDevice *_this);
    void (*free)(SDL_VideoDevice *_this);

    /* Data common to all drivers */
    int num_displays;
    SDL_VideoDisplay **displays;
    SDL_Window *windows;
    void *internal;
};

typedef struct VideoBootStrap {
    const char *name;
    const char *desc;
    SDL_VideoDevice *(*create)(void);
    bool (*ShowMessageBox)(const SDL_MessageBoxData *messageboxdata, int *buttonID);
} VideoBootStrap;

extern SDL_VideoDevice *SDL_GetVideoDevice(void);
extern SDL_DisplayID SDL_AddBasicVideoDisplay(const SDL_DisplayMode *desktop_mode);

#endif /* SDL_sysvideo_h_ */
