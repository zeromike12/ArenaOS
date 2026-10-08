/*
 * ArenaOS C2 minimal stdio subset (EXPERIMENTAL). Scope: docs/compat/C2-LIBC-COVERAGE.md.
 *
 * Streams: stdin, stdout, and stderr are the GRANTED StandardStreamSet (ADR-0104).
 * They are not ambient filesystem authority. fopen/freopen/remove/rename are
 * refused with errno ENOSYS: general file access needs a broker interface (BLK-C2-03).
 * Output is unbuffered: every call completes or reports an error before returning.
 * Floating-point conversions are refused (errno ENOTSUP), never printed as text.
 */
#ifndef ARENA_LIBC_STDIO_H
#define ARENA_LIBC_STDIO_H

#include <stdarg.h>
#include <stddef.h>

#define EOF (-1)
#define BUFSIZ 128
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

typedef struct arena_file FILE;

extern FILE *const stdin;
extern FILE *const stdout;
extern FILE *const stderr;

int printf(const char *fmt, ...);
int vprintf(const char *fmt, va_list ap);
int fprintf(FILE *f, const char *fmt, ...);
int vfprintf(FILE *f, const char *fmt, va_list ap);
int sprintf(char *buf, const char *fmt, ...);
int vsprintf(char *buf, const char *fmt, va_list ap);
int snprintf(char *buf, size_t n, const char *fmt, ...);
int vsnprintf(char *buf, size_t n, const char *fmt, va_list ap);

int puts(const char *s);
int fputs(const char *s, FILE *f);
int putchar(int c);
int fputc(int c, FILE *f);
int putc(int c, FILE *f);
int getchar(void);
int fgetc(FILE *f);
int getc(FILE *f);
char *fgets(char *buf, int n, FILE *f);
size_t fwrite(const void *p, size_t size, size_t n, FILE *f);
size_t fread(void *p, size_t size, size_t n, FILE *f);
int fflush(FILE *f);
int fclose(FILE *f);
int ferror(FILE *f);
int feof(FILE *f);
void clearerr(FILE *f);
int setvbuf(FILE *f, char *buf, int mode, size_t size);
void perror(const char *s);

FILE *fopen(const char *path, const char *mode);
FILE *freopen(const char *path, const char *mode, FILE *f);
int remove(const char *path);
int rename(const char *from, const char *to);

#endif /* ARENA_LIBC_STDIO_H */
