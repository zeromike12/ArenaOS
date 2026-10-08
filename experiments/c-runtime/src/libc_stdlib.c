/*
 * C2.2 stdlib subset. Memory goes ONLY through the C1 allocator (alloc.c). There
 * is no second allocator. Integer conversions are exact; no floating point.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/rt.h"
#include "arena/string.h"
#include "libc/errno.h"
#include "libc_impl.h"
#include "libc_internal.h"

/* LP64: long is 64 bits on x86_64 ArenaOS. */
#define LONG_MAX_ARENA ((long)INT64_MAX)
#define LONG_MIN_ARENA ((long)INT64_MIN)
#define ULONG_MAX_ARENA ((unsigned long)UINT64_MAX)


/* Standard wrappers: a NULL return sets errno (C2 contract). */
void *arena_std_malloc(size_t n) {
    void *p = arena_malloc(n);
    if (p == NULL) {
        arena_c_set_errno(ENOMEM);
    }
    return p;
}

void *arena_std_calloc(size_t count, size_t size) {
    void *p = arena_calloc(count, size);
    if (p == NULL) {
        arena_c_set_errno(ENOMEM); /* overflow or allocation refusal */
    }
    return p;
}

void *arena_std_realloc(void *p, size_t n) {
    void *q = arena_realloc(p, n);
    if (q == NULL && n != 0) {
        arena_c_set_errno(ENOMEM); /* the original block is untouched on failure */
    }
    return q;
}

void arena_std_free(void *p) { arena_free(p); }

void *arena_std_aligned_alloc(size_t alignment, size_t size) {
    void *p = arena_aligned_alloc(alignment, size);
    if (p == NULL) {
        /* Not a power of two: EINVAL. Otherwise a refusal (limit, overflow, or memory). */
        int bad = alignment == 0 || (alignment & (alignment - 1)) != 0;
        arena_c_set_errno(bad ? EINVAL : ENOMEM);
    }
    return p;
}

int arena_abs(int v) { return v < 0 ? -v : v; }

long arena_labs(long v) { return v < 0 ? -v : v; }

long long arena_llabs(long long v) { return v < 0 ? -v : v; }

static int digit_value(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'z') return c - 'a' + 10;
    if (c >= 'A' && c <= 'Z') return c - 'A' + 10;
    return 99;
}

/* Exact integer parse. Returns the magnitude; sets *negative and *overflow.
 * Base 0 accepts 0x and 0 prefixes. Bases 2..36 otherwise. */
static uint64_t parse_magnitude(const char *s, char **end, int base, int *negative, int *overflow,
                                int *invalid, uint64_t limit) {
    const char *p = s;
    *negative = 0;
    *overflow = 0;
    *invalid = 0;
    while (*p == ' ' || (*p >= '\t' && *p <= '\r')) {
        p++;
    }
    if (*p == '+' || *p == '-') {
        *negative = *p == '-';
        p++;
    }
    if ((base == 0 || base == 16) && p[0] == '0' && (p[1] == 'x' || p[1] == 'X') &&
        digit_value(p[2]) < 16) {
        base = 16;
        p += 2;
    } else if (base == 0) {
        base = (p[0] == '0') ? 8 : 10;
    }
    if (base < 2 || base > 36) {
        *invalid = 1;
        if (end) *end = (char *)s;
        return 0;
    }
    uint64_t value = 0;
    int any = 0;
    for (;; p++) {
        int d = digit_value(*p);
        if (d >= base) {
            break;
        }
        any = 1;
        if (!*overflow) {
            if (value > (limit - (uint64_t)d) / (uint64_t)base) {
                *overflow = 1;
            } else {
                value = value * (uint64_t)base + (uint64_t)d;
            }
        }
    }
    if (end) {
        *end = (char *)(any ? p : s);
    }
    if (!any) {
        *invalid = 1;
    }
    return value;
}

long long arena_strtoll(const char *s, char **end, int base) {
    int neg, ovf, inv;
    uint64_t lim = (uint64_t)INT64_MAX + 1u; /* allow |INT64_MIN| magnitude when negative */
    uint64_t mag = parse_magnitude(s, end, base, &neg, &ovf, &inv, lim);
    if (inv) {
        return 0;
    }
    if (ovf || (!neg && mag > (uint64_t)INT64_MAX) || (neg && mag > lim)) {
        arena_c_set_errno(34); /* ERANGE */
        return neg ? INT64_MIN : INT64_MAX;
    }
    return neg ? (long long)(0u - mag) : (long long)mag;
}

unsigned long long arena_strtoull(const char *s, char **end, int base) {
    int neg, ovf, inv;
    uint64_t mag = parse_magnitude(s, end, base, &neg, &ovf, &inv, UINT64_MAX);
    if (inv) {
        return 0;
    }
    if (ovf) {
        arena_c_set_errno(34);
        return UINT64_MAX;
    }
    return neg ? 0u - (unsigned long long)mag : (unsigned long long)mag;
}

long arena_strtol(const char *s, char **end, int base) {
    long long v = arena_strtoll(s, end, base);
    if (v > LONG_MAX_ARENA || v < LONG_MIN_ARENA) {
        arena_c_set_errno(34);
        return v > 0 ? LONG_MAX_ARENA : LONG_MIN_ARENA;
    }
    return (long)v;
}

unsigned long arena_strtoul(const char *s, char **end, int base) {
    unsigned long long v = arena_strtoull(s, end, base);
    if (v > ULONG_MAX_ARENA) {
        arena_c_set_errno(34);
        return ULONG_MAX_ARENA;
    }
    return (unsigned long)v;
}

int arena_atoi(const char *s) { return (int)arena_strtol(s, NULL, 10); }
long arena_atol(const char *s) { return arena_strtol(s, NULL, 10); }
long long arena_atoll(const char *s) { return arena_strtoll(s, NULL, 10); }

static void swap_bytes(unsigned char *a, unsigned char *b, size_t size) {
    for (size_t i = 0; i < size; i++) {
        unsigned char t = a[i];
        a[i] = b[i];
        b[i] = t;
    }
}

/* Heapsort on byte-sized elements: O(n log n), in place, no allocation. */
static void sift_down(unsigned char *base, size_t size, size_t start, size_t end,
                      int (*cmp)(const void *, const void *)) {
    size_t root = start;
    for (;;) {
        size_t child = 2 * root + 1;
        if (child >= end) {
            return;
        }
        if (child + 1 < end && cmp(base + child * size, base + (child + 1) * size) < 0) {
            child++;
        }
        if (cmp(base + root * size, base + child * size) >= 0) {
            return;
        }
        swap_bytes(base + root * size, base + child * size, size);
        root = child;
    }
}

void arena_qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *)) {
    if (n < 2 || size == 0 || cmp == NULL) {
        return;
    }
    unsigned char *b = (unsigned char *)base;
    for (size_t start = n / 2; start-- > 0;) {
        sift_down(b, size, start, n, cmp);
    }
    for (size_t end = n - 1; end > 0; end--) {
        swap_bytes(b, b + end * size, size);
        sift_down(b, size, 0, end, cmp);
    }
}

void *arena_bsearch(const void *key, const void *base, size_t n, size_t size,
                    int (*cmp)(const void *, const void *)) {
    const unsigned char *b = (const unsigned char *)base;
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        int c = cmp(key, b + mid * size);
        if (c == 0) {
            return (void *)(b + mid * size);
        }
        if (c < 0) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    return NULL;
}

static uint32_t rand_state = 1u;

int arena_rand(void) {
    rand_state = rand_state * 1103515245u + 12345u;
    return (int)((rand_state >> 16) & 0x7fffu);
}

void arena_srand(unsigned int seed) { rand_state = seed; }

/* Environment: the granted startup record's envp, read-only. */
char *arena_getenv(const char *name) {
    const arena_startup_t *s = arena_startup();
    if (!s->present || name == NULL || name[0] == '\0') {
        return NULL;
    }
    size_t nl = arena_strlen(name);
    for (int i = 0; i < s->envc; i++) {
        const char *e = s->envp[i];
        if (arena_strncmp(e, name, nl) == 0 && e[nl] == '=') {
            return (char *)(e + nl + 1);
        }
    }
    return NULL;
}

/* Integer division (C11 7.22.6). Division by zero is a caller error: the result is
 * undefined by the standard; here it returns zeros rather than trapping. */
#include "libc/stdlib.h"
div_t arena_div(int a, int b) {
    div_t r = {0, 0};
    if (b != 0) {
        r.quot = a / b;
        r.rem = a % b;
    }
    return r;
}

ldiv_t arena_ldiv(long a, long b) {
    ldiv_t r = {0, 0};
    if (b != 0) {
        r.quot = a / b;
        r.rem = a % b;
    }
    return r;
}

lldiv_t arena_lldiv(long long a, long long b) {
    lldiv_t r = {0, 0};
    if (b != 0) {
        r.quot = a / b;
        r.rem = a % b;
    }
    return r;
}
