/*
 * Internal declarations for the arena_* implementations of the C2 libc subset.
 * Not installed in the SDK. The standard names come from include/libc/ headers, and
 * src/libc_names.c (guest build only) aliases each standard name to the
 * implementation declared here. docs/compat/C2-LIBC-COVERAGE.md lists the mapping.
 */
#ifndef ARENA_LIBC_IMPL_H
#define ARENA_LIBC_IMPL_H

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include "libc/stdlib.h"
#include "libc/time.h"

struct arena_file;

/* string.c and libc_string.c */
size_t arena_strnlen(const char *s, size_t max);
int arena_strncmp(const char *a, const char *b, size_t n);
int arena_strcoll(const char *a, const char *b);
char *arena_strcpy(char *dst, const char *src);
char *arena_strncpy(char *dst, const char *src, size_t n);
char *arena_strcat(char *dst, const char *src);
char *arena_strncat(char *dst, const char *src, size_t n);
char *arena_strchr(const char *s, int c);
char *arena_strrchr(const char *s, int c);
char *arena_strstr(const char *hay, const char *needle);
char *arena_strpbrk(const char *s, const char *accept);
size_t arena_strspn(const char *s, const char *accept);
size_t arena_strcspn(const char *s, const char *reject);
char *arena_strtok(char *s, const char *delim);
void *arena_memchr(const void *s, int c, size_t n);
char *arena_strerror(int errnum);

/* stdio.c (formatter, FILE layer) */
int arena_fputs(const char *s, struct arena_file *f);
int arena_puts(const char *s);
int arena_fputc(int c, struct arena_file *f);
int arena_putc(int c, struct arena_file *f);
int arena_putchar(int c);
int arena_fgetc(struct arena_file *f);
int arena_getc(struct arena_file *f);
int arena_getchar(void);
int arena_ungetc(int c, struct arena_file *f);
char *arena_fgets(char *buf, int n, struct arena_file *f);
size_t arena_fwrite(const void *p, size_t size, size_t n, struct arena_file *f);
size_t arena_fread(void *p, size_t size, size_t n, struct arena_file *f);
int arena_fflush(struct arena_file *f);
int arena_fclose(struct arena_file *f);
int arena_ferror(struct arena_file *f);
int arena_feof(struct arena_file *f);
void arena_clearerr(struct arena_file *f);
int arena_setvbuf(struct arena_file *f, char *buf, int mode, size_t size);
void arena_perror(const char *s);
struct arena_file *arena_fopen(const char *path, const char *mode);
struct arena_file *arena_freopen(const char *path, const char *mode, struct arena_file *f);
int arena_remove(const char *path);
int arena_rename(const char *from, const char *to);
int arena_vfprintf(struct arena_file *f, const char *fmt, va_list ap);
int arena_fprintf(struct arena_file *f, const char *fmt, ...);
int arena_vsprintf(char *buf, const char *fmt, va_list ap);
int arena_sprintf(char *buf, const char *fmt, ...);

/* libc_stdlib.c */
void *arena_std_malloc(size_t n);
void *arena_std_calloc(size_t count, size_t size);
void *arena_std_realloc(void *p, size_t n);
void arena_std_free(void *p);
void *arena_std_aligned_alloc(size_t alignment, size_t size);
void arena_libc_exit(int status) __attribute__((noreturn));
void arena_libc_Exit(int status) __attribute__((noreturn));
void arena_libc_abort(void) __attribute__((noreturn));
int arena_atexit(void (*fn)(void));
char *arena_getenv(const char *name);
int arena_abs(int v);
long arena_labs(long v);
long long arena_llabs(long long v);
long arena_strtol(const char *s, char **end, int base);
unsigned long arena_strtoul(const char *s, char **end, int base);
long long arena_strtoll(const char *s, char **end, int base);
unsigned long long arena_strtoull(const char *s, char **end, int base);
int arena_atoi(const char *s);
div_t arena_div(int a, int b);
ldiv_t arena_ldiv(long a, long b);
lldiv_t arena_lldiv(long long a, long long b);
long arena_atol(const char *s);
long long arena_atoll(const char *s);
void arena_qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
void *arena_bsearch(const void *key, const void *base, size_t n, size_t size,
                    int (*cmp)(const void *, const void *));
int arena_rand(void);
void arena_srand(unsigned int seed);

/* libc_time.c */
int arena_clock_gettime(clockid_t id, struct timespec *ts);
int arena_nanosleep(const struct timespec *req, struct timespec *rem);
time_t arena_time(time_t *out);
clock_t arena_clock(void);

#endif /* ARENA_LIBC_IMPL_H */
