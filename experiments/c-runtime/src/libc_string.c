/*
 * C2.2 string functions beyond the C1 core (string.c). Standard semantics within
 * the C locale. Every function bounds its reads by the caller's sizes, except
 * the NUL-terminated string arguments, which the caller must provide (as in C).
 */
#include <stddef.h>

#include "arena/string.h"
#include "libc_impl.h"
#include "libc_internal.h"



int arena_strcoll(const char *a, const char *b) { return arena_strcmp(a, b); }

char *arena_strcpy(char *dst, const char *src) {
    char *d = dst;
    while ((*d++ = *src++) != '\0') {
    }
    return dst;
}

char *arena_strncpy(char *dst, const char *src, size_t n) {
    size_t i = 0;
    for (; i < n && src[i] != '\0'; i++) {
        dst[i] = src[i];
    }
    for (; i < n; i++) {
        dst[i] = '\0'; /* standard: pad with NULs to n */
    }
    return dst;
}

char *arena_strcat(char *dst, const char *src) {
    arena_strcpy(dst + arena_strlen(dst), src);
    return dst;
}

char *arena_strncat(char *dst, const char *src, size_t n) {
    size_t d = arena_strlen(dst);
    size_t i = 0;
    for (; i < n && src[i] != '\0'; i++) {
        dst[d + i] = src[i];
    }
    dst[d + i] = '\0';
    return dst;
}

char *arena_strchr(const char *s, int c) {
    char ch = (char)c;
    for (;; s++) {
        if (*s == ch) {
            return (char *)s;
        }
        if (*s == '\0') {
            return NULL;
        }
    }
}

char *arena_strrchr(const char *s, int c) {
    char ch = (char)c;
    const char *last = NULL;
    for (;; s++) {
        if (*s == ch) {
            last = s;
        }
        if (*s == '\0') {
            return (char *)last;
        }
    }
}

char *arena_strstr(const char *hay, const char *needle) {
    size_t nl = arena_strlen(needle);
    if (nl == 0) {
        return (char *)hay;
    }
    for (; *hay != '\0'; hay++) {
        if (hay[0] == needle[0] && arena_strncmp(hay, needle, nl) == 0) {
            return (char *)hay;
        }
    }
    return NULL;
}

size_t arena_strspn(const char *s, const char *accept) {
    size_t n = 0;
    while (s[n] != '\0' && arena_strchr(accept, s[n]) != NULL) {
        n++;
    }
    return n;
}

size_t arena_strcspn(const char *s, const char *reject) {
    size_t n = 0;
    while (s[n] != '\0' && arena_strchr(reject, s[n]) == NULL) {
        n++;
    }
    return n;
}

char *arena_strpbrk(const char *s, const char *accept) {
    for (; *s != '\0'; s++) {
        if (arena_strchr(accept, *s) != NULL) {
            return (char *)s;
        }
    }
    return NULL;
}

/* Not thread-safe (static cursor). Kept for C-library source compatibility. */
static char *strtok_cursor;

char *arena_strtok(char *s, const char *delim) {
    if (s == NULL) {
        s = strtok_cursor;
    }
    if (s == NULL) {
        return NULL;
    }
    s += arena_strspn(s, delim);
    if (*s == '\0') {
        strtok_cursor = NULL;
        return NULL;
    }
    char *end = s + arena_strcspn(s, delim);
    if (*end == '\0') {
        strtok_cursor = NULL;
    } else {
        *end = '\0';
        strtok_cursor = end + 1;
    }
    return s;
}

void *arena_memchr(const void *s, int c, size_t n) {
    const unsigned char *p = (const unsigned char *)s;
    unsigned char ch = (unsigned char)c;
    for (size_t i = 0; i < n; i++) {
        if (p[i] == ch) {
            return (void *)(p + i);
        }
    }
    return NULL;
}

char *arena_strerror(int errnum) { return (char *)arena_strerror_text(errnum); }
