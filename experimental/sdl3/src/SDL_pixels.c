#include "SDL_internal.h"

static const SDL_PixelFormatDetails s_format_xrgb8888 = {
    SDL_PIXELFORMAT_XRGB8888, 32, 4, {0, 0},
    0x00FF0000, 0x0000FF00, 0x000000FF, 0x00000000,
    8, 8, 8, 0,
    16, 8, 0, 0
};

static const SDL_PixelFormatDetails s_format_argb8888 = {
    SDL_PIXELFORMAT_ARGB8888, 32, 4, {0, 0},
    0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000,
    8, 8, 8, 8,
    16, 8, 0, 24
};

static const SDL_PixelFormatDetails s_format_rgbx8888 = {
    SDL_PIXELFORMAT_RGBX8888, 32, 4, {0, 0},
    0xFF000000, 0x00FF0000, 0x0000FF00, 0x00000000,
    8, 8, 8, 0,
    24, 16, 8, 0
};

static const SDL_PixelFormatDetails s_format_rgba8888 = {
    SDL_PIXELFORMAT_RGBA8888, 32, 4, {0, 0},
    0xFF000000, 0x00FF0000, 0x0000FF00, 0x000000FF,
    8, 8, 8, 8,
    24, 16, 8, 0
};

const SDL_PixelFormatDetails * SDLCALL SDL_GetPixelFormatDetails(SDL_PixelFormat format) {
    switch (format) {
        case SDL_PIXELFORMAT_XRGB8888:
            return &s_format_xrgb8888;
        case SDL_PIXELFORMAT_ARGB8888:
            return &s_format_argb8888;
        case SDL_PIXELFORMAT_RGBX8888:
            return &s_format_rgbx8888;
        case SDL_PIXELFORMAT_RGBA8888:
            return &s_format_rgba8888;
        default:
            return &s_format_xrgb8888;
    }
}

const char * SDLCALL SDL_GetPixelFormatName(SDL_PixelFormat format) {
    switch (format) {
        case SDL_PIXELFORMAT_XRGB8888: return "SDL_PIXELFORMAT_XRGB8888";
        case SDL_PIXELFORMAT_ARGB8888: return "SDL_PIXELFORMAT_ARGB8888";
        case SDL_PIXELFORMAT_RGBX8888: return "SDL_PIXELFORMAT_RGBX8888";
        case SDL_PIXELFORMAT_RGBA8888: return "SDL_PIXELFORMAT_RGBA8888";
        default: return "SDL_PIXELFORMAT_UNKNOWN";
    }
}

Uint32 SDLCALL SDL_MapRGB(const SDL_PixelFormatDetails *format, const SDL_Palette *palette, Uint8 r, Uint8 g, Uint8 b) {
    (void)palette;
    if (!format) return 0;
    if (format->format == SDL_PIXELFORMAT_XRGB8888 || format->format == SDL_PIXELFORMAT_ARGB8888) {
        return ((Uint32)r << 16) | ((Uint32)g << 8) | ((Uint32)b);
    }
    return ((Uint32)(r >> (8 - format->Rbits)) << format->Rshift) |
           ((Uint32)(g >> (8 - format->Gbits)) << format->Gshift) |
           ((Uint32)(b >> (8 - format->Bbits)) << format->Bshift);
}

Uint32 SDLCALL SDL_MapRGBA(const SDL_PixelFormatDetails *format, const SDL_Palette *palette, Uint8 r, Uint8 g, Uint8 b, Uint8 a) {
    (void)palette;
    if (!format) return 0;
    if (format->format == SDL_PIXELFORMAT_XRGB8888) {
        return ((Uint32)r << 16) | ((Uint32)g << 8) | ((Uint32)b);
    }
    if (format->format == SDL_PIXELFORMAT_ARGB8888) {
        return ((Uint32)a << 24) | ((Uint32)r << 16) | ((Uint32)g << 8) | ((Uint32)b);
    }
    return ((Uint32)(r >> (8 - format->Rbits)) << format->Rshift) |
           ((Uint32)(g >> (8 - format->Gbits)) << format->Gshift) |
           ((Uint32)(b >> (8 - format->Bbits)) << format->Bshift) |
           ((Uint32)(a >> (8 - format->Abits)) << format->Ashift);
}

void SDLCALL SDL_GetRGB(Uint32 pixel, const SDL_PixelFormatDetails *format, const SDL_Palette *palette, Uint8 *r, Uint8 *g, Uint8 *b) {
    (void)palette;
    if (!format) return;
    if (r) *r = (Uint8)((pixel & format->Rmask) >> format->Rshift);
    if (g) *g = (Uint8)((pixel & format->Gmask) >> format->Gshift);
    if (b) *b = (Uint8)((pixel & format->Bmask) >> format->Bshift);
}

void SDLCALL SDL_GetRGBA(Uint32 pixel, const SDL_PixelFormatDetails *format, const SDL_Palette *palette, Uint8 *r, Uint8 *g, Uint8 *b, Uint8 *a) {
    (void)palette;
    if (!format) return;
    if (r) *r = (Uint8)((pixel & format->Rmask) >> format->Rshift);
    if (g) *g = (Uint8)((pixel & format->Gmask) >> format->Gshift);
    if (b) *b = (Uint8)((pixel & format->Bmask) >> format->Bshift);
    if (a) {
        if (format->Amask) {
            *a = (Uint8)((pixel & format->Amask) >> format->Ashift);
        } else {
            *a = 255;
        }
    }
}
