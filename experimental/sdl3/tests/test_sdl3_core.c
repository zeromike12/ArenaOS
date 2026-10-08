#include <SDL3/SDL.h>
#include <stdio.h>
#include <assert.h>

int main(void) {
    printf("=== Running SDL3 Core Unit Tests ===\n");

    /* 1. Test Format Details */
    const SDL_PixelFormatDetails *details = SDL_GetPixelFormatDetails(SDL_PIXELFORMAT_XRGB8888);
    assert(details != NULL);
    assert(details->bits_per_pixel == 32);
    assert(details->bytes_per_pixel == 4);
    assert(details->Rmask == 0x00FF0000);
    assert(details->Gmask == 0x0000FF00);
    assert(details->Bmask == 0x000000FF);
    printf("[PASS] Pixel format details verified\n");

    /* 2. Test Pixel Mapping */
    Uint32 mapped = SDL_MapRGB(details, NULL, 0x12, 0x34, 0x56);
    assert(mapped == 0x00123456);

    Uint8 r = 0, g = 0, b = 0;
    SDL_GetRGB(mapped, details, NULL, &r, &g, &b);
    assert(r == 0x12);
    assert(g == 0x34);
    assert(b == 0x56);
    printf("[PASS] Pixel map/get verified\n");

    /* 3. Test Rect Utilities */
    SDL_Rect r1 = {10, 10, 50, 50};
    SDL_Rect r2 = {30, 30, 50, 50};
    SDL_Rect r3 = {100, 100, 20, 20};

    assert(SDL_HasRectIntersection(&r1, &r2) == true);
    assert(SDL_HasRectIntersection(&r1, &r3) == false);

    SDL_Rect inter;
    assert(SDL_GetRectIntersection(&r1, &r2, &inter) == true);
    assert(inter.x == 30 && inter.y == 30 && inter.w == 30 && inter.h == 30);

    SDL_Rect uni;
    assert(SDL_GetRectUnion(&r1, &r2, &uni) == true);
    assert(uni.x == 10 && uni.y == 10 && uni.w == 70 && uni.h == 70);
    printf("[PASS] Rect intersections and unions verified\n");

    /* 4. Test Surface Creation and Filling */
    SDL_Surface *surface = SDL_CreateSurface(100, 100, SDL_PIXELFORMAT_XRGB8888);
    assert(surface != NULL);
    assert(surface->w == 100);
    assert(surface->h == 100);
    assert(surface->pitch >= 400);
    assert(surface->pixels != NULL);

    /* Fill background */
    assert(SDL_FillSurfaceRect(surface, NULL, 0x00112233) == true);
    Uint32 *px = (Uint32 *)surface->pixels;
    assert(px[0] == 0x00112233);
    assert(px[99 * 100 + 99] == 0x00112233);

    /* Fill subrect */
    SDL_Rect sub = {10, 10, 20, 20};
    assert(SDL_FillSurfaceRect(surface, &sub, 0x00AABBCC) == true);
    assert(px[10 * 100 + 10] == 0x00AABBCC);
    assert(px[29 * 100 + 29] == 0x00AABBCC);
    assert(px[30 * 100 + 30] == 0x00112233);

    SDL_DestroySurface(surface);
    printf("[PASS] Surface creation, filling, and destruction verified\n");

    printf("=== All SDL3 Core Unit Tests PASSED ===\n");
    return 0;
}
