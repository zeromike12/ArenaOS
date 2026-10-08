/* Freestanding string/memory primitives for the ArenaOS C prototype. */
#ifndef ARENA_STRING_H
#define ARENA_STRING_H

#include <stddef.h>

void *arena_memcpy(void *dst, const void *src, size_t n);
void *arena_memmove(void *dst, const void *src, size_t n);
void *arena_memset(void *dst, int c, size_t n);
int arena_memcmp(const void *a, const void *b, size_t n);
size_t arena_strlen(const char *s);
size_t arena_strnlen(const char *s, size_t max);
int arena_strcmp(const char *a, const char *b);
int arena_strncmp(const char *a, const char *b, size_t n);

#endif /* ARENA_STRING_H */
