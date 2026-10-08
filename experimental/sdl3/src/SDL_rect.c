#include "SDL_internal.h"

bool SDLCALL SDL_HasRectIntersection(const SDL_Rect *A, const SDL_Rect *B) {
    if (!A || !B) return false;
    int AminX = A->x;
    int AmaxX = A->x + A->w;
    int AminY = A->y;
    int AmaxY = A->y + A->h;

    int BminX = B->x;
    int BmaxX = B->x + B->w;
    int BminY = B->y;
    int BmaxY = B->y + B->h;

    if (BminX >= AmaxX || BmaxX <= AminX || BminY >= AmaxY || BmaxY <= AminY) {
        return false;
    }
    return true;
}

bool SDLCALL SDL_GetRectIntersection(const SDL_Rect *A, const SDL_Rect *B, SDL_Rect *result) {
    if (!A || !B) return false;
    int AminX = A->x;
    int AmaxX = A->x + A->w;
    int AminY = A->y;
    int AmaxY = A->y + A->h;

    int BminX = B->x;
    int BmaxX = B->x + B->w;
    int BminY = B->y;
    int BmaxY = B->y + B->h;

    int resMinX = (AminX > BminX) ? AminX : BminX;
    int resMaxX = (AmaxX < BmaxX) ? AmaxX : BmaxX;
    int resMinY = (AminY > BminY) ? AminY : BminY;
    int resMaxY = (AmaxY < BmaxY) ? AmaxY : BmaxY;

    if (resMinX < resMaxX && resMinY < resMaxY) {
        if (result) {
            result->x = resMinX;
            result->y = resMinY;
            result->w = resMaxX - resMinX;
            result->h = resMaxY - resMinY;
        }
        return true;
    }
    if (result) {
        result->x = 0;
        result->y = 0;
        result->w = 0;
        result->h = 0;
    }
    return false;
}

bool SDLCALL SDL_GetRectUnion(const SDL_Rect *A, const SDL_Rect *B, SDL_Rect *result) {
    if (!result) return false;
    if (!A && !B) return false;
    if (!A) {
        *result = *B;
        return true;
    }
    if (!B) {
        *result = *A;
        return true;
    }

    int minX = (A->x < B->x) ? A->x : B->x;
    int minY = (A->y < B->y) ? A->y : B->y;
    int maxX = ((A->x + A->w) > (B->x + B->w)) ? (A->x + A->w) : (B->x + B->w);
    int maxY = ((A->y + A->h) > (B->y + B->h)) ? (A->y + A->h) : (B->y + B->h);

    result->x = minX;
    result->y = minY;
    result->w = maxX - minX;
    result->h = maxY - minY;
    return true;
}

bool SDLCALL SDL_GetRectEnclosingPoints(const SDL_Point *points, int count, const SDL_Rect *clip, SDL_Rect *result) {
    if (!points || count <= 0 || !result) return false;
    int minX = 0x7fffffff, minY = 0x7fffffff;
    int maxX = -0x7fffffff, maxY = -0x7fffffff;
    int added = 0;

    for (int i = 0; i < count; i++) {
        int x = points[i].x;
        int y = points[i].y;
        if (clip) {
            if (x < clip->x || x >= clip->x + clip->w ||
                y < clip->y || y >= clip->y + clip->h) {
                continue;
            }
        }
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
        added++;
    }

    if (added == 0) return false;
    result->x = minX;
    result->y = minY;
    result->w = maxX - minX + 1;
    result->h = maxY - minY + 1;
    return true;
}
