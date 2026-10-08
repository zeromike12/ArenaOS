/*
 * Bounded printf subset (prototype).
 *
 * Supported: %% %c %s %d %i %u %x %X %o %p with the flags '-' and '0', a
 * decimal width, a '.' precision (strings only), and length modifiers
 * hh h l ll z j t. Unsupported conversions are echoed verbatim so output
 * never silently loses text. No floating point, no '*', no positional args.
 *
 * Output goes either to a caller buffer (snprintf family, always
 * NUL-terminated when n > 0) or, for printf, through a 128-byte buffer that
 * is flushed to fd 1 as it fills.
 */
#include <stdarg.h>
#include <stdint.h>

#include "arena/rt.h"
#include "arena/string.h"

struct sink {
    char *out;   /* snprintf target, or NULL for fd output */
    size_t cap;  /* snprintf capacity including the terminator */
    size_t count; /* bytes the formatted text WOULD occupy */
    int fd;      /* output fd for printf, -1 for snprintf */
    size_t used; /* bytes pending in fbuf */
    char fbuf[128];
};

static void sink_flush(struct sink *s) {
    if (s->fd >= 0 && s->used > 0) {
        arena_write(s->fd, s->fbuf, s->used);
        s->used = 0;
    }
}

static void sink_put(struct sink *s, char c) {
    if (s->fd >= 0) {
        if (s->used == sizeof s->fbuf) {
            sink_flush(s);
        }
        s->fbuf[s->used++] = c;
    } else if (s->out != NULL && s->cap > 0 && s->count + 1 < s->cap) {
        s->out[s->count] = c;
    }
    s->count++;
}

static void sink_put_n(struct sink *s, const char *p, size_t n) {
    for (size_t i = 0; i < n; i++) {
        sink_put(s, p[i]);
    }
}

static void emit_field(struct sink *s, const char *prefix, size_t nprefix,
                       const char *digits, size_t ndigits, size_t width,
                       int left, int zero) {
    size_t total = nprefix + ndigits;
    size_t pad = width > total ? width - total : 0;
    if (!left && !zero) {
        for (size_t i = 0; i < pad; i++) {
            sink_put(s, ' ');
        }
    }
    sink_put_n(s, prefix, nprefix);
    if (!left && zero) {
        for (size_t i = 0; i < pad; i++) {
            sink_put(s, '0');
        }
    }
    sink_put_n(s, digits, ndigits);
    if (left) {
        for (size_t i = 0; i < pad; i++) {
            sink_put(s, ' ');
        }
    }
}

static void emit_unsigned(struct sink *s, uint64_t v, unsigned base, int upper,
                          const char *prefix, size_t width, int left, int zero) {
    static const char lo[] = "0123456789abcdef";
    static const char hi[] = "0123456789ABCDEF";
    const char *table = upper ? hi : lo;
    char digits[64];
    size_t n = 0;
    do {
        digits[n++] = table[v % base];
        v /= base;
    } while (v != 0);
    char reversed[64];
    for (size_t i = 0; i < n; i++) {
        reversed[i] = digits[n - 1 - i];
    }
    emit_field(s, prefix, prefix ? arena_strlen(prefix) : 0, reversed, n, width, left, zero);
}

static void emit_signed(struct sink *s, int64_t v, size_t width, int left, int zero) {
    uint64_t mag = v < 0 ? (uint64_t)0 - (uint64_t)v : (uint64_t)v;
    emit_unsigned(s, mag, 10, 0, v < 0 ? "-" : "", width, left, zero);
}

static int vformat(struct sink *s, const char *fmt, va_list ap) {
    for (const char *p = fmt; *p != '\0'; p++) {
        if (*p != '%') {
            sink_put(s, *p);
            continue;
        }
        const char *spec_start = p;
        p++;
        int left = 0;
        int zero = 0;
        for (;; p++) {
            if (*p == '-') {
                left = 1;
            } else if (*p == '0') {
                zero = 1;
            } else {
                break;
            }
        }
        size_t width = 0;
        while (*p >= '0' && *p <= '9') {
            width = width * 10 + (size_t)(*p - '0');
            p++;
        }
        size_t precision = (size_t)-1;
        if (*p == '.') {
            p++;
            precision = 0;
            while (*p >= '0' && *p <= '9') {
                precision = precision * 10 + (size_t)(*p - '0');
                p++;
            }
        }
        int lng = 0; /* 0 = int, 1 = long-sized, 2 = char, 3 = short */
        if (*p == 'h') {
            p++;
            lng = 3;
            if (*p == 'h') {
                p++;
                lng = 2;
            }
        } else if (*p == 'l') {
            p++;
            lng = 1;
            if (*p == 'l') {
                p++;
            }
        } else if (*p == 'z' || *p == 'j' || *p == 't') {
            p++;
            lng = 1;
        }
        char conv = *p;
        if (conv == '\0') {
            sink_put_n(s, spec_start, arena_strlen(spec_start));
            break;
        }
        switch (conv) {
        case '%':
            sink_put(s, '%');
            break;
        case 'c':
            sink_put(s, (char)va_arg(ap, int));
            break;
        case 's': {
            const char *str = va_arg(ap, const char *);
            if (str == NULL) {
                str = "(null)";
            }
            size_t len = precision == (size_t)-1 ? arena_strlen(str) : arena_strnlen(str, precision);
            emit_field(s, "", 0, str, len, width, left, 0);
            break;
        }
        case 'd':
        case 'i': {
            int64_t v;
            if (lng == 1) {
                v = va_arg(ap, long);
            } else if (lng == 2 || lng == 3) {
                v = (int64_t)va_arg(ap, int);
                if (lng == 3) {
                    v = (int16_t)v;
                } else {
                    v = (int8_t)v;
                }
            } else {
                v = va_arg(ap, int);
            }
            emit_signed(s, v, width, left, zero);
            break;
        }
        case 'u':
        case 'x':
        case 'X':
        case 'o': {
            uint64_t v;
            if (lng == 1) {
                v = va_arg(ap, unsigned long);
            } else if (lng == 2) {
                v = (uint8_t)va_arg(ap, unsigned int);
            } else if (lng == 3) {
                v = (uint16_t)va_arg(ap, unsigned int);
            } else {
                v = va_arg(ap, unsigned int);
            }
            unsigned base = conv == 'u' ? 10u : (conv == 'o' ? 8u : 16u);
            emit_unsigned(s, v, base, conv == 'X', NULL, width, left, zero);
            break;
        }
        case 'p': {
            uintptr_t v = (uintptr_t)va_arg(ap, void *);
            emit_unsigned(s, (uint64_t)v, 16, 0, "0x", width, left, zero);
            break;
        }
        default:
            sink_put_n(s, spec_start, (size_t)(p - spec_start) + 1);
            break;
        }
    }
    return 0;
}

int arena_vsnprintf(char *buf, size_t n, const char *fmt, va_list ap) {
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

int arena_vprintf(const char *fmt, va_list ap) {
    struct sink s = {0};
    s.fd = 1;
    vformat(&s, fmt, ap);
    sink_flush(&s);
    return (int)s.count;
}

int arena_printf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vprintf(fmt, ap);
    va_end(ap);
    return r;
}
