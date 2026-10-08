#include "SDL_internal.h"

SDL_Surface * SDLCALL SDL_CreateSurface(int width, int height, SDL_PixelFormat format) {
    if (width <= 0 || height <= 0) {
        SDL_SetError("Invalid surface dimensions");
        return NULL;
    }

    const SDL_PixelFormatDetails *details = SDL_GetPixelFormatDetails(format);
    if (!details) {
        SDL_SetError("Unsupported pixel format");
        return NULL;
    }

    int pitch = width * details->bytes_per_pixel;
    /* 4-byte align pitch */
    pitch = (pitch + 3) & ~3;

    size_t size = (size_t)height * (size_t)pitch;
    void *pixels = SDL_malloc(size);
    if (!pixels) {
        SDL_OutOfMemory();
        return NULL;
    }
    SDL_memset(pixels, 0, size);

    SDL_Surface *surface = (SDL_Surface *)SDL_calloc(1, sizeof(SDL_Surface));
    if (!surface) {
        SDL_free(pixels);
        SDL_OutOfMemory();
        return NULL;
    }

    surface->format = format;
    surface->w = width;
    surface->h = height;
    surface->pitch = pitch;
    surface->pixels = pixels;
    surface->refcount = 1;
    surface->flags = 0;

    return surface;
}

SDL_Surface * SDLCALL SDL_CreateSurfaceFrom(int width, int height, SDL_PixelFormat format, void *pixels, int pitch) {
    if (width <= 0 || height <= 0 || !pixels || pitch <= 0) {
        SDL_SetError("Invalid arguments to SDL_CreateSurfaceFrom");
        return NULL;
    }

    SDL_Surface *surface = (SDL_Surface *)SDL_calloc(1, sizeof(SDL_Surface));
    if (!surface) {
        SDL_OutOfMemory();
        return NULL;
    }

    surface->format = format;
    surface->w = width;
    surface->h = height;
    surface->pitch = pitch;
    surface->pixels = pixels;
    surface->refcount = 1;
    surface->flags = SDL_SURFACE_PREALLOCATED;

    return surface;
}

void SDLCALL SDL_DestroySurface(SDL_Surface *surface) {
    if (!surface) return;
    if (--surface->refcount > 0) return;

    if (!(surface->flags & SDL_SURFACE_PREALLOCATED) && surface->pixels) {
        SDL_free(surface->pixels);
        surface->pixels = NULL;
    }
    SDL_free(surface);
}

bool SDLCALL SDL_LockSurface(SDL_Surface *surface) {
    (void)surface;
    return true;
}

void SDLCALL SDL_UnlockSurface(SDL_Surface *surface) {
    (void)surface;
}

bool SDLCALL SDL_FillSurfaceRect(SDL_Surface *dst, const SDL_Rect *rect, Uint32 color) {
    if (!dst || !dst->pixels) {
        return SDL_SetError("Invalid surface for fill");
    }

    SDL_Rect r;
    if (rect) {
        r = *rect;
        if (r.x < 0) { r.w += r.x; r.x = 0; }
        if (r.y < 0) { r.h += r.y; r.y = 0; }
        if (r.x + r.w > dst->w) r.w = dst->w - r.x;
        if (r.y + r.h > dst->h) r.h = dst->h - r.y;
        if (r.w <= 0 || r.h <= 0) return true;
    } else {
        r.x = 0;
        r.y = 0;
        r.w = dst->w;
        r.h = dst->h;
    }

    const SDL_PixelFormatDetails *details = SDL_GetPixelFormatDetails(dst->format);
    int bpp = details ? details->bytes_per_pixel : 4;

    if (bpp == 4) {
        for (int y = 0; y < r.h; y++) {
            Uint32 *row = (Uint32 *)((Uint8 *)dst->pixels + (r.y + y) * dst->pitch) + r.x;
            for (int x = 0; x < r.w; x++) {
                row[x] = color;
            }
        }
    } else {
        /* Fallback for generic formats */
        for (int y = 0; y < r.h; y++) {
            Uint8 *row = (Uint8 *)dst->pixels + (r.y + y) * dst->pitch + r.x * bpp;
            for (int x = 0; x < r.w; x++) {
                SDL_memcpy(row + x * bpp, &color, bpp);
            }
        }
    }

    return true;
}

bool SDLCALL SDL_FillSurfaceRects(SDL_Surface *dst, const SDL_Rect *rects, int count, Uint32 color) {
    if (!dst || !rects || count <= 0) return false;
    for (int i = 0; i < count; i++) {
        if (!SDL_FillSurfaceRect(dst, &rects[i], color)) {
            return false;
        }
    }
    return true;
}

bool SDLCALL SDL_BlitSurface(SDL_Surface *src, const SDL_Rect *srcrect, SDL_Surface *dst, const SDL_Rect *dstrect) {
    if (!src || !src->pixels || !dst || !dst->pixels) {
        return SDL_SetError("Invalid surfaces for blit");
    }

    SDL_Rect s = srcrect ? *srcrect : (SDL_Rect){0, 0, src->w, src->h};
    SDL_Rect d = dstrect ? *dstrect : (SDL_Rect){0, 0, dst->w, dst->h};

    /* Clip */
    if (s.x < 0) { s.w += s.x; d.x -= s.x; s.x = 0; }
    if (s.y < 0) { s.h += s.y; d.y -= s.y; s.y = 0; }
    if (s.x + s.w > src->w) s.w = src->w - s.x;
    if (s.y + s.h > src->h) s.h = src->h - s.y;

    if (d.x < 0) { s.x -= d.x; s.w += d.x; d.w += d.x; d.x = 0; }
    if (d.y < 0) { s.y -= d.y; s.h += d.y; d.h += d.y; d.y = 0; }
    if (d.x + s.w > dst->w) s.w = dst->w - d.x;
    if (d.y + s.h > dst->h) s.h = dst->h - d.y;

    if (s.w <= 0 || s.h <= 0) return true;

    int bytes_per_pixel = 4;
    for (int y = 0; y < s.h; y++) {
        const Uint8 *src_row = (const Uint8 *)src->pixels + (s.y + y) * src->pitch + s.x * bytes_per_pixel;
        Uint8 *dst_row = (Uint8 *)dst->pixels + (d.y + y) * dst->pitch + d.x * bytes_per_pixel;
        SDL_memcpy(dst_row, src_row, s.w * bytes_per_pixel);
    }

    return true;
}
