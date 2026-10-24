/*
 * ArenaOS C2 minimal stdlib subset (EXPERIMENTAL). Scope: docs/compat/C2-LIBC-COVERAGE.md.
 *
 * Memory: every allocation goes through the C1 VM-backed buddy allocator. There is
 * no second allocator. Floating-point conversions (atof, strtod) are not provided.
 */
#ifndef ARENA_LIBC_STDLIB_H
#define ARENA_LIBC_STDLIB_H

#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX 32767

typedef struct { int quot; int rem; } div_t;
typedef struct { long quot; long rem; } ldiv_t;
typedef struct { long long quot; long long rem; } lldiv_t;

void *malloc(size_t n);
void *calloc(size_t count, size_t size);
void *realloc(void *p, size_t n);
void free(void *p);
void *aligned_alloc(size_t alignment, size_t size);

__attribute__((noreturn)) void exit(int status);
__attribute__((noreturn)) void _Exit(int status);
__attribute__((noreturn)) void abort(void);
int atexit(void (*fn)(void));

char *getenv(const char *name);

int abs(int v);
long labs(long v);
long long llabs(long long v);
div_t div(int a, int b);
ldiv_t ldiv(long a, long b);
lldiv_t lldiv(long long a, long long b);

int atoi(const char *s);
long atol(const char *s);
long long atoll(const char *s);
long strtol(const char *s, char **end, int base);
unsigned long strtoul(const char *s, char **end, int base);
long long strtoll(const char *s, char **end, int base);
unsigned long long strtoull(const char *s, char **end, int base);

void qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
void *bsearch(const void *key, const void *base, size_t n, size_t size,
              int (*cmp)(const void *, const void *));

int rand(void);
void srand(unsigned int seed);

#endif /* ARENA_LIBC_STDLIB_H */
