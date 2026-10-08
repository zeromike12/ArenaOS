/*
 * ArenaOS C runtime stdio (EXPERIMENTAL, C1.1 formatter; C2.2 FILE layer).
 *
 * Output: fd 1/2 are the granted StandardStreamSet (arena_write -> streams.c).
 * Every formatted call first runs a DRY pass over the format string. Any
 * unsupported conversion (floating point, %n, unknown) is refused BEFORE a byte
 * is written: the call returns -1 with errno set (ENOTSUP for floating point,
 * EINVAL for %n and unknown). Nothing is ever printed as literal text.
 *
 * Return values: printf-family functions return the byte count on success, or a
 * negative value on error (the first negative ARENA_E_* from the write path, or
 * -1 for a refused conversion). snprintf-family returns the count the full text
 * would occupy, or -1 on refusal.
 */
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include "arena/rt.h"
#include "arena/string.h"
#include "libc_internal.h"

extern struct arena_file *const arena_stdin_var;
extern struct arena_file *const arena_stdout_var;
extern struct arena_file *const arena_stderr_var;

#define ARENA_C_EINVAL 22
#define ARENA_C_ENOTSUP 95

struct arena_file {
    int fd;       /* granted stream index: 0 stdin, 1 stdout, 2 stderr */
    int err;      /* sticky error flag (ferror) */
    int eof;      /* sticky end-of-file flag (feof) */
    int open;     /* 1 while the stream is usable */
    int ungot;    /* 1 when a pushed-back byte is pending */
    unsigned char ungot_byte;
};

static struct arena_file s_stdin = {0, 0, 0, 1, 0, 0};
static struct arena_file s_stdout = {1, 0, 0, 1, 0, 0};
static struct arena_file s_stderr = {2, 0, 0, 1, 0, 0};

/* Constant pointers used by the standard-name aliases (libc_names.c). */
struct arena_file *const arena_stdin_var = &s_stdin;
struct arena_file *const arena_stdout_var = &s_stdout;
struct arena_file *const arena_stderr_var = &s_stderr;

#ifndef ARENA_HOSTED
/* The standard stream objects (guest build; host tests use the arena_* names). */
struct arena_file *const stdin = &s_stdin;
struct arena_file *const stdout = &s_stdout;
struct arena_file *const stderr = &s_stderr;
#endif

struct arena_file *arena_stdin_object(void) { return &s_stdin; }
struct arena_file *arena_stdout_object(void) { return &s_stdout; }
struct arena_file *arena_stderr_object(void) { return &s_stderr; }

struct sink {
    char *out;      /* snprintf target, or NULL */
    size_t cap;     /* snprintf capacity including the terminator */
    size_t count;   /* bytes the formatted text occupies (or would occupy) */
    int fd;         /* output fd, or -1 for memory and dry runs */
    int dry;        /* 1: count and validate only */
    size_t used;    /* bytes pending in fbuf */
    int err;        /* first write error (negative), 0 if none */
    int refused;    /* errno value for a refused conversion, 0 if none */
    char fbuf[128];
};

static void sink_flush(struct sink *s) {
    if (s->fd >= 0 && s->used > 0 && !s->dry) {
        size_t off = 0;
        while (off < s->used && s->err == 0) {
            long n = arena_write(s->fd, s->fbuf + off, s->used - off);
            if (n < 0) {
                s->err = (int)n;
            } else if (n == 0) {
                s->err = ARENA_E_CLOSED;
            } else {
                off += (size_t)n;
            }
        }
    }
    s->used = 0;
}

static void sink_put_n(struct sink *s, const char *p, size_t n) {
    s->count += n;
    if (s->dry) {
        return;
    }
    if (s->fd >= 0) {
        while (n > 0) {
            size_t room = sizeof(s->fbuf) - s->used;
            size_t take = n < room ? n : room;
            arena_memcpy(s->fbuf + s->used, p, take);
            s->used += take;
            p += take;
            n -= take;
            if (s->used == sizeof(s->fbuf)) {
                sink_flush(s);
            }
        }
        return;
    }
    if (s->out != NULL && s->cap > 0) {
        size_t room = s->cap - 1;
        size_t before = s->count - n;
        if (before < room) {
            size_t take = n < room - before ? n : room - before;
            arena_memcpy(s->out + before, p, take);
        }
    }
}

static void sink_put(struct sink *s, char c) { sink_put_n(s, &c, 1); }

static void sink_repeat(struct sink *s, char c, size_t n) {
    while (n-- > 0) {
        sink_put(s, c);
    }
}

/* Emit one field: optional sign or prefix, then digits with precision zeros,
 * padded to `width`. `zero_pad` applies only when no precision was given. */
static void emit_field(struct sink *s, const char *prefix, size_t nprefix,
                       const char *digits, size_t ndigits, size_t precision_zeros,
                       size_t width, int left, int zero_pad) {
    size_t total = nprefix + precision_zeros + ndigits;
    size_t pad = width > total ? width - total : 0;
    if (!left && !zero_pad) {
        sink_repeat(s, ' ', pad);
    }
    sink_put_n(s, prefix, nprefix);
    if (!left && zero_pad) {
        sink_repeat(s, '0', pad);
    }
    sink_repeat(s, '0', precision_zeros);
    sink_put_n(s, digits, ndigits);
    if (left) {
        sink_repeat(s, ' ', pad);
    }
}

static size_t format_unsigned(char *buf, uint64_t v, unsigned base, int upper) {
    const char *dig = upper ? "0123456789ABCDEF" : "0123456789abcdef";
    char tmp[24];
    size_t n = 0;
    do {
        tmp[n++] = dig[v % base];
        v /= base;
    } while (v != 0);
    for (size_t i = 0; i < n; i++) {
        buf[i] = tmp[n - 1 - i];
    }
    return n;
}

/* Walk the format string. In dry mode `s->dry` is set and nothing is written.
 * On a refused conversion, sets s->refused and stops. */
static void vformat(struct sink *s, const char *fmt, va_list ap) {
    for (const char *p = fmt; *p != '\0' && s->refused == 0; p++) {
        if (*p != '%') {
            sink_put(s, *p);
            continue;
        }
        const char *spec_start = p;
        p++;
        if (*p == '%') {
            sink_put(s, '%');
            continue;
        }
        int left = 0, zero = 0, plus = 0, space = 0, alt = 0;
        for (;; p++) {
            if (*p == '-') left = 1;
            else if (*p == '0') zero = 1;
            else if (*p == '+') plus = 1;
            else if (*p == ' ') space = 1;
            else if (*p == '#') alt = 1;
            else break;
        }
        size_t width = 0;
        if (*p == '*') {
            int w = va_arg(ap, int);
            if (w < 0) {
                left = 1;
                w = -w;
            }
            width = (size_t)w;
            p++;
        } else {
            while (*p >= '0' && *p <= '9') {
                width = width * 10 + (size_t)(*p - '0');
                p++;
            }
        }
        size_t precision = (size_t)-1;
        if (*p == '.') {
            p++;
            precision = 0;
            if (*p == '*') {
                int pr = va_arg(ap, int);
                precision = pr < 0 ? (size_t)-1 : (size_t)pr;
                p++;
            } else {
                while (*p >= '0' && *p <= '9') {
                    precision = precision * 10 + (size_t)(*p - '0');
                    p++;
                }
            }
        }
        /* length: 0 int, 1 long, 2 long long, 3 char, 4 short, 5 size/intmax/ptrdiff */
        int len = 0;
        if (*p == 'h') {
            p++;
            len = 4;
            if (*p == 'h') {
                p++;
                len = 3;
            }
        } else if (*p == 'l') {
            p++;
            len = 1;
            if (*p == 'l') {
                p++;
                len = 2;
            }
        } else if (*p == 'z' || *p == 'j' || *p == 't') {
            p++;
            len = 5;
        } else if (*p == 'L') {
            s->refused = ARENA_C_ENOTSUP; /* long double */
            return;
        }
        char conv = *p;
        if (conv == '\0') {
            /* Trailing '%': echoed, as C1 did for unknown conversions. */
            sink_put_n(s, spec_start, (size_t)(p - spec_start));
            return;
        }
        switch (conv) {
        case 'c': {
            char c = (char)va_arg(ap, int);
            emit_field(s, "", 0, &c, 1, 0, width, left, 0);
            break;
        }
        case 's': {
            const char *str = va_arg(ap, const char *);
            if (str == NULL) {
                str = "(null)";
            }
            size_t n = 0;
            while ((precision == (size_t)-1 || n < precision) && str[n] != '\0') {
                n++;
            }
            emit_field(s, "", 0, str, n, 0, width, left, 0);
            break;
        }
        case 'd':
        case 'i': {
            int64_t v;
            if (len == 1 || len == 5) {
                v = (int64_t)va_arg(ap, long);
            } else if (len == 2) {
                v = (int64_t)va_arg(ap, long long);
            } else {
                int iv = va_arg(ap, int);
                v = len == 3 ? (int64_t)(signed char)iv : len == 4 ? (int64_t)(short)iv : (int64_t)iv;
            }
            char prefix[2];
            size_t np = 0;
            uint64_t mag;
            if (v < 0) {
                prefix[np++] = '-';
                mag = (uint64_t)0 - (uint64_t)v;
            } else {
                if (plus) prefix[np++] = '+';
                else if (space) prefix[np++] = ' ';
                mag = (uint64_t)v;
            }
            char digits[24];
            size_t nd = format_unsigned(digits, mag, 10, 0);
            if (precision == 0 && mag == 0) {
                nd = 0;
            }
            size_t pz = (precision != (size_t)-1 && precision > nd) ? precision - nd : 0;
            emit_field(s, prefix, np, digits, nd, pz, width, left,
                       zero && precision == (size_t)-1);
            break;
        }
        case 'u':
        case 'x':
        case 'X':
        case 'o': {
            uint64_t v;
            if (len == 1 || len == 5) {
                v = (uint64_t)va_arg(ap, unsigned long);
            } else if (len == 2) {
                v = (uint64_t)va_arg(ap, unsigned long long);
            } else {
                unsigned int uv = va_arg(ap, unsigned int);
                v = len == 3 ? (uint64_t)(unsigned char)uv
                    : len == 4 ? (uint64_t)(unsigned short)uv
                               : (uint64_t)uv;
            }
            unsigned base = conv == 'u' ? 10u : conv == 'o' ? 8u : 16u;
            char digits[24];
            size_t nd = format_unsigned(digits, v, base, conv == 'X');
            if (precision == 0 && v == 0) {
                nd = 0;
            }
            char prefix[2];
            size_t np = 0;
            if (alt && v != 0 && conv == 'x') {
                prefix[np++] = '0';
                prefix[np++] = 'x';
            } else if (alt && v != 0 && conv == 'X') {
                prefix[np++] = '0';
                prefix[np++] = 'X';
            }
            size_t pz = (precision != (size_t)-1 && precision > nd) ? precision - nd : 0;
            if (alt && conv == 'o' && pz == 0 && (nd == 0 || digits[0] != '0')) {
                pz = 1;
            }
            emit_field(s, prefix, np, digits, nd, pz, width, left,
                       zero && precision == (size_t)-1);
            break;
        }
        case 'p': {
            void *ptr = va_arg(ap, void *);
            if (ptr == NULL) {
                emit_field(s, "", 0, "(nil)", 5, 0, width, left, 0);
            } else {
                char digits[24];
                size_t nd = format_unsigned(digits, (uint64_t)(uintptr_t)ptr, 16, 0);
                emit_field(s, "0x", 2, digits, nd, 0, width, left, zero);
            }
            break;
        }
        case 'f':
        case 'F':
        case 'e':
        case 'E':
        case 'g':
        case 'G':
        case 'a':
        case 'A':
            /* Floating point is not supported by the C2 profile. Refused, never faked. */
            s->refused = ARENA_C_ENOTSUP;
            return;
        case 'n':
            s->refused = ARENA_C_EINVAL; /* %n writes through an argument; refused */
            return;
        default:
            /* Unknown conversion: the spec text is echoed and no argument is consumed
             * (C1 contract, pinned by host test "unknown conversion echo"). */
            sink_put_n(s, spec_start, (size_t)(p - spec_start) + 1);
            break;
        }
    }
}

/* Dry pass: validate the whole format string without output. */
static int format_check(const char *fmt, va_list ap) {
    struct sink probe = {0};
    probe.fd = -1;
    probe.dry = 1;
    va_list copy;
    va_copy(copy, ap);
    vformat(&probe, fmt, copy);
    va_end(copy);
    return probe.refused;
}

static int format_to_fd(int fd, const char *fmt, va_list ap) {
    int refused = format_check(fmt, ap);
    if (refused != 0) {
        arena_c_set_errno(refused);
        return -1;
    }
    struct sink s = {0};
    s.fd = fd;
    vformat(&s, fmt, ap);
    sink_flush(&s);
    if (s.err != 0) {
        arena_c_set_errno(arena_c_errno_for(s.err));
        return s.err;
    }
    return (int)s.count;
}

int arena_vsnprintf(char *buf, size_t n, const char *fmt, va_list ap) {
    int refused = format_check(fmt, ap);
    if (refused != 0) {
        arena_c_set_errno(refused);
        return -1;
    }
    struct sink s = {0};
    s.out = buf;
    s.cap = n;
    s.fd = -1;
    vformat(&s, fmt, ap);
    if (buf != NULL && n > 0) {
        size_t end = s.count < n ? s.count : n - 1;
        buf[end] = '\0';
    }
    return (int)s.count;
}

int arena_snprintf(char *buf, size_t n, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vsnprintf(buf, n, fmt, ap);
    va_end(ap);
    return r;
}

/* printf to the granted stdout (fd 1). */
int arena_vprintf(const char *fmt, va_list ap) { return format_to_fd(1, fmt, ap); }

int arena_printf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vprintf(fmt, ap);
    va_end(ap);
    return r;
}

/* --- FILE layer (arena_* names; libc_names.c exports the standard names) --- */

#define ARENA_C_EBADF 9
#define ARENA_C_EPIPE 32
#define ARENA_C_EOVERFLOW 75
#define ARENA_C_ENOSYS 38

static int file_usable(struct arena_file *f) {
    if (f == NULL || !f->open) {
        arena_c_set_errno(ARENA_C_EBADF);
        return 0;
    }
    return 1;
}

static int file_writable(struct arena_file *f) {
    if (!file_usable(f)) {
        return 0;
    }
    if (f->fd == 0) {
        arena_c_set_errno(ARENA_C_EBADF); /* stdin is not writable */
        return 0;
    }
    return 1;
}

static int file_readable(struct arena_file *f) {
    if (!file_usable(f)) {
        return 0;
    }
    if (f->fd != 0) {
        arena_c_set_errno(ARENA_C_EBADF); /* stdout and stderr are not readable */
        return 0;
    }
    return 1;
}

/* Write all bytes or report the first error. Returns 0 or a negative status. */
static long write_all(struct arena_file *f, const void *p, size_t n) {
    size_t off = 0;
    while (off < n) {
        long w = arena_write(f->fd, (const char *)p + off, n - off);
        if (w < 0) {
            f->err = 1;
            arena_c_set_errno(arena_c_errno_for(w));
            return w;
        }
        if (w == 0) {
            f->err = 1;
            arena_c_set_errno(ARENA_C_EPIPE);
            return ARENA_E_CLOSED;
        }
        off += (size_t)w;
    }
    return 0;
}

/* Read one byte from the granted stream. Returns 1, 0 at end of file, or negative. */
static long read_one(struct arena_file *f, unsigned char *out) {
    if (f->ungot) {
        f->ungot = 0;
        *out = f->ungot_byte;
        return 1;
    }
    for (;;) {
        long r = arena_stream_read(f->fd, out, 1);
        if (r == ARENA_E_WOULD_BLOCK) {
            continue;
        }
        if (r < 0) {
            f->err = 1;
            arena_c_set_errno(arena_c_errno_for(r));
            return r;
        }
        if (r == 0) {
            f->eof = 1;
        }
        return r;
    }
}

int arena_fputs(const char *s, struct arena_file *f) {
    if (s == NULL || !file_writable(f)) {
        return -1;
    }
    return write_all(f, s, arena_strlen(s)) == 0 ? 0 : -1;
}

int arena_puts(const char *s) {
    if (arena_fputs(s, arena_stdout_var) != 0) {
        return -1;
    }
    return arena_fputs("\n", arena_stdout_var);
}

int arena_fputc(int c, struct arena_file *f) {
    unsigned char b = (unsigned char)c;
    if (!file_writable(f)) {
        return -1;
    }
    return write_all(f, &b, 1) == 0 ? (int)b : -1;
}

int arena_putc(int c, struct arena_file *f) { return arena_fputc(c, f); }

int arena_putchar(int c) { return arena_fputc(c, arena_stdout_var); }

int arena_fgetc(struct arena_file *f) {
    unsigned char b;
    if (!file_readable(f)) {
        return -1;
    }
    return read_one(f, &b) == 1 ? (int)b : -1;
}

int arena_getc(struct arena_file *f) { return arena_fgetc(f); }

int arena_getchar(void) { return arena_fgetc(arena_stdin_var); }

int arena_ungetc(int c, struct arena_file *f) {
    if (c < 0 || c > 255 || !file_readable(f) || f->ungot) {
        return -1;
    }
    f->ungot = 1;
    f->ungot_byte = (unsigned char)c;
    f->eof = 0;
    return c;
}

char *arena_fgets(char *buf, int n, struct arena_file *f) {
    if (buf == NULL || n <= 0 || !file_readable(f)) {
        return NULL;
    }
    int i = 0;
    while (i < n - 1) {
        unsigned char b;
        long r = read_one(f, &b);
        if (r <= 0) {
            break;
        }
        buf[i++] = (char)b;
        if (b == '\n') {
            break;
        }
    }
    if (i == 0) {
        return NULL;
    }
    buf[i] = '\0';
    return buf;
}

size_t arena_fwrite(const void *p, size_t size, size_t n, struct arena_file *f) {
    if (size == 0 || n == 0) {
        return 0;
    }
    if (!file_writable(f)) {
        return 0;
    }
    if (n > ((size_t)-1) / size) {
        arena_c_set_errno(ARENA_C_EOVERFLOW);
        return 0;
    }
    if (write_all(f, p, size * n) != 0) {
        return 0;
    }
    return n;
}

size_t arena_fread(void *p, size_t size, size_t n, struct arena_file *f) {
    if (size == 0 || n == 0) {
        return 0;
    }
    if (!file_readable(f)) {
        return 0;
    }
    if (n > ((size_t)-1) / size) {
        arena_c_set_errno(ARENA_C_EOVERFLOW);
        return 0;
    }
    size_t total = size * n;
    unsigned char *out = (unsigned char *)p;
    size_t done = 0;
    while (done < total) {
        unsigned char b;
        long r = read_one(f, &b);
        if (r <= 0) {
            break;
        }
        out[done++] = b;
    }
    return done / size;
}

/* Output is unbuffered: fflush has nothing pending to write. */
int arena_fflush(struct arena_file *f) {
    if (f == NULL) {
        return 0;
    }
    if (!f->open) {
        arena_c_set_errno(ARENA_C_EBADF);
        return -1;
    }
    return 0;
}

int arena_fclose(struct arena_file *f) {
    if (!file_usable(f)) {
        return -1;
    }
    long r = arena_stream_close(f->fd);
    f->open = 0;
    if (r < 0) {
        arena_c_set_errno(arena_c_errno_for(r));
        return -1;
    }
    return 0;
}

int arena_ferror(struct arena_file *f) { return f != NULL && f->err != 0; }

int arena_feof(struct arena_file *f) { return f != NULL && f->eof != 0; }

void arena_clearerr(struct arena_file *f) {
    if (f != NULL) {
        f->err = 0;
        f->eof = 0;
    }
}

/* Accepted for compatibility. Every stream is unbuffered, which satisfies any
 * requested mode. The caller's buffer is never used. */
int arena_setvbuf(struct arena_file *f, char *buf, int mode, size_t size) {
    (void)buf;
    (void)size;
    if (!file_usable(f) || mode < 0 || mode > 2) {
        arena_c_set_errno(ARENA_C_EINVAL);
        return -1;
    }
    return 0;
}

void arena_perror(const char *s) {
    const char *msg = arena_strerror_text(arena_c_get_errno());
    if (s != NULL && s[0] != '\0') {
        arena_fputs(s, arena_stderr_var);
        arena_fputs(": ", arena_stderr_var);
    }
    arena_fputs(msg, arena_stderr_var);
    arena_fputs("\n", arena_stderr_var);
}

/* File access is not granted by the C2 profile (BLK-C2-03). */
struct arena_file *arena_fopen(const char *path, const char *mode) {
    (void)path;
    (void)mode;
    arena_c_set_errno(ARENA_C_ENOSYS);
    return NULL;
}

struct arena_file *arena_freopen(const char *path, const char *mode, struct arena_file *f) {
    (void)path;
    (void)mode;
    (void)f;
    arena_c_set_errno(ARENA_C_ENOSYS);
    return NULL;
}

int arena_remove(const char *path) {
    (void)path;
    arena_c_set_errno(ARENA_C_ENOSYS);
    return -1;
}

int arena_rename(const char *from, const char *to) {
    (void)from;
    (void)to;
    arena_c_set_errno(ARENA_C_ENOSYS);
    return -1;
}

int arena_vfprintf(struct arena_file *f, const char *fmt, va_list ap) {
    if (!file_writable(f)) {
        return -1;
    }
    return format_to_fd(f->fd, fmt, ap);
}

int arena_fprintf(struct arena_file *f, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vfprintf(f, fmt, ap);
    va_end(ap);
    return r;
}

/* sprintf is UNBOUNDED by definition. Prefer snprintf. */
int arena_vsprintf(char *buf, const char *fmt, va_list ap) {
    return arena_vsnprintf(buf, (size_t)-1, fmt, ap);
}

int arena_sprintf(char *buf, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vsprintf(buf, fmt, ap);
    va_end(ap);
    return r;
}
