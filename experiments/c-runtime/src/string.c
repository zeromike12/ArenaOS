/*
 * Freestanding memory and string primitives (prototype).
 *
 * memcpy/memset/memmove use the x86-64 string instructions. The loop-based
 * functions carry optnone/no-idiom attributes: without them the optimizer
 * may recognise the loop as a memcpy/memset/bcmp idiom and emit a call back
 * to the very symbol being defined, which would recurse forever.
 */
#include <stdint.h>

#include "arena/string.h"

#if defined(__clang__)
#define ARENA_NO_IDIOM __attribute__((noinline, optnone))
#else
#define ARENA_NO_IDIOM __attribute__((noinline, optimize("no-tree-loop-distribute-patterns", "no-tree-vectorize")))
#endif

void *arena_memcpy(void *dst, const void *src, size_t n) {
    void *ret = dst;
    __asm__ volatile("rep movsb"
                     : "+D"(dst), "+S"(src), "+c"(n)
                     :
                     : "memory");
    return ret;
}

void *arena_memset(void *dst, int c, size_t n) {
    void *ret = dst;
    __asm__ volatile("rep stosb"
                     : "+D"(dst), "+c"(n)
                     : "a"(c)
                     : "memory");
    return ret;
}

/* Overlap-safe: copy forward when the destination is below the source (or
 * the ranges are disjoint); otherwise copy backward with DF set. DF is
 * always cleared again before returning (ABI requirement). */
void *arena_memmove(void *dst, const void *src, size_t n) {
    void *ret = dst;
    if (n == 0 || dst == src) {
        return ret;
    }
    if ((uintptr_t)dst < (uintptr_t)src || (uintptr_t)dst >= (uintptr_t)src + n) {
        __asm__ volatile("rep movsb"
                         : "+D"(dst), "+S"(src), "+c"(n)
                         :
                         : "memory");
    } else {
        unsigned char *d = (unsigned char *)dst + n - 1;
        const unsigned char *s = (const unsigned char *)src + n - 1;
        __asm__ volatile("std\n\trep movsb\n\tcld"
                         : "+D"(d), "+S"(s), "+c"(n)
                         :
                         : "memory", "cc");
    }
    return ret;
}

ARENA_NO_IDIOM int arena_memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *x = (const unsigned char *)a;
    const unsigned char *y = (const unsigned char *)b;
    for (size_t i = 0; i < n; i++) {
        if (x[i] != y[i]) {
            return (int)x[i] - (int)y[i];
        }
    }
    return 0;
}

ARENA_NO_IDIOM size_t arena_strlen(const char *s) {
    size_t n = 0;
    while (s[n] != '\0') {
        n++;
    }
    return n;
}

ARENA_NO_IDIOM size_t arena_strnlen(const char *s, size_t max) {
    size_t n = 0;
    while (n < max && s[n] != '\0') {
        n++;
    }
    return n;
}

ARENA_NO_IDIOM int arena_strcmp(const char *a, const char *b) {
    size_t i = 0;
    for (;; i++) {
        unsigned char x = (unsigned char)a[i];
        unsigned char y = (unsigned char)b[i];
        if (x != y) {
            return (int)x - (int)y;
        }
        if (x == 0) {
            return 0;
        }
    }
}

ARENA_NO_IDIOM int arena_strncmp(const char *a, const char *b, size_t n) {
    for (size_t i = 0; i < n; i++) {
        unsigned char x = (unsigned char)a[i];
        unsigned char y = (unsigned char)b[i];
        if (x != y) {
            return (int)x - (int)y;
        }
        if (x == 0) {
            return 0;
        }
    }
    return 0;
}

