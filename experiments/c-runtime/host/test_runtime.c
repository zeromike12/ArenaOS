/*
 * HOST-ONLY test suite for the ArenaOS C runtime prototype.
 *
 * Runs the runtime's allocator, formatter and string/memory primitives on a
 * Linux host (mmap backend) under AddressSanitizer and UBSan. Expectations
 * come from glibc, a shadow model, or hand-written literals - never from the
 * functions under test. These results prove the code's logic on the host.
 * They say NOTHING about guest execution; that is the GUEST evidence class.
 */
#define _GNU_SOURCE
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "arena/rt.h"
#include "arena/string.h"

static int checks;
static int failures;

#define CHECK(cond, ...)                                                  \
    do {                                                                  \
        checks++;                                                         \
        if (!(cond)) {                                                    \
            failures++;                                                   \
            fprintf(stderr, "FAIL %s:%d: %s -- ", __FILE__, __LINE__, #cond); \
            fprintf(stderr, __VA_ARGS__);                                 \
            fprintf(stderr, "\n");                                        \
        }                                                                 \
    } while (0)

/* Deterministic xorshift PRNG: failures are reproducible from the seed. */
static uint64_t rng_state = 0x9E3779B97F4A7C15ull;
static uint64_t rnd(void) {
    uint64_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    return rng_state = x;
}

/* ---- formatting: differential against glibc snprintf -------------------- */

static void test_format_differential(void) {
    enum { N = 20000 };
    static const char *flags[] = {"", "-", "0", "-0"};
    static const char *dmods[] = {"", "hh", "h", "l", "ll", "z"};
    int bad = 0;
    for (int i = 0; i < N; i++) {
        char fmt[64];
        char want[512], got[512];
        int w = (int)(rnd() % 25);
        const char *f = flags[rnd() % 4];
        int kind = (int)(rnd() % 5);
        int64_t sv = (int64_t)rnd();
        uint64_t uv = rnd();
        if (rnd() % 3 == 0) {
            sv &= 0xFFFF;
            uv &= 0xFFFF;
        }
        if (kind == 0) {
            const char *m = dmods[rnd() % 6];
            /* glibc and the runtime both narrow %hh/%h before printing. */
            if (m[0] == 'l' || m[0] == 'z') {
                snprintf(fmt, sizeof fmt, "%%%s%d%sd", f, w, m);
                snprintf(want, sizeof want, fmt, (long long)sv);
                arena_snprintf(got, sizeof got, fmt, (long long)sv);
            } else {
                snprintf(fmt, sizeof fmt, "%%%s%d%sd", f, w, m);
                snprintf(want, sizeof want, fmt, (int)sv);
                arena_snprintf(got, sizeof got, fmt, (int)sv);
            }
        } else if (kind == 1) {
            snprintf(fmt, sizeof fmt, "%%%s%d%s", f, w, rnd() % 2 ? "x" : "X");
            snprintf(want, sizeof want, fmt, (unsigned)uv);
            arena_snprintf(got, sizeof got, fmt, (unsigned)uv);
        } else if (kind == 2) {
            snprintf(fmt, sizeof fmt, "%%%s%dlu", f, w);
            snprintf(want, sizeof want, fmt, (unsigned long)uv);
            arena_snprintf(got, sizeof got, fmt, (unsigned long)uv);
        } else if (kind == 3) {
            snprintf(fmt, sizeof fmt, "%%%s%dllx", f, w);
            snprintf(want, sizeof want, fmt, (unsigned long long)uv);
            arena_snprintf(got, sizeof got, fmt, (unsigned long long)uv);
        } else {
            /* string with width and precision */
            char src[40];
            size_t len = rnd() % 30;
            for (size_t k = 0; k < len; k++) {
                src[k] = (char)('a' + (rnd() % 26));
            }
            src[len] = '\0';
            int prec = (int)(rnd() % 12);
            /* glibc's zero-flag behaviour for %s is not portable; use spaces. */
            const char *sf = rnd() % 2 ? "" : "-";
            snprintf(fmt, sizeof fmt, "%%%s%d.%ds", sf, w, prec);
            snprintf(want, sizeof want, fmt, src);
            arena_snprintf(got, sizeof got, fmt, src);
        }
        if (strcmp(want, got) != 0) {
            bad++;
            if (bad < 5) {
                fprintf(stderr, "  fmt '%s': want '%s' got '%s'\n", fmt, want, got);
            }
        }
    }
    CHECK(bad == 0, "%d of %d formatted outputs differed from glibc", bad, N);
}

static void test_format_fixed(void) {
    char b[256];
    int n = arena_snprintf(b, sizeof b, "%s|%c|%%|%5.2s|%-3d|%03u", "x", 'y', "hello", 7, 5u);
    CHECK(n == 19 && strcmp(b, "x|y|%|   he|7  |005") == 0, "fixed format got '%s' (%d)", b, n);
    /* unknown conversion is echoed, not lost */
    n = arena_snprintf(b, sizeof b, "a%qb");
    CHECK(strcmp(b, "a%qb") == 0, "unknown conversion echo got '%s'", b);
    /* NULL destination and zero capacity: only counts */
    n = arena_snprintf(NULL, 0, "%d%d", 12, 34);
    CHECK(n == 4, "count-only snprintf returned %d", n);
    char tiny[1];
    n = arena_snprintf(tiny, sizeof tiny, "abc");
    CHECK(n == 3 && tiny[0] == '\0', "1-byte buffer must hold only NUL");
}

/* ---- string and memory primitives ------------------------------------- */

static void test_memory_string(void) {
    enum { MAXN = 4096 };
    static unsigned char a[MAXN + 64], b[MAXN + 64], c[MAXN + 64];
    int bad_move = 0, bad_cmp = 0, bad_set = 0;
    for (int iter = 0; iter < 5000; iter++) {
        size_t n = rnd() % MAXN;
        size_t off_src = rnd() % 64, off_dst = rnd() % 64;
        for (size_t i = 0; i < sizeof a; i++) {
            a[i] = (unsigned char)rnd();
        }
        memcpy(b, a, sizeof a);
        memcpy(c, a, sizeof a);
        arena_memmove(b + off_dst, b + off_src, n);
        memmove(c + off_dst, c + off_src, n);
        if (memcmp(b, c, sizeof b) != 0) {
            bad_move++;
        }
        int want = memcmp(a + off_src, a + off_dst, n);
        int got = arena_memcmp(a + off_src, a + off_dst, n);
        if ((want < 0) != (got < 0) || (want > 0) != (got > 0)) {
            bad_cmp++;
        }
        int c8 = (int)(rnd() & 0xFF);
        arena_memset(b, c8, n);
        memset(c, c8, n);
        if (memcmp(b, c, n) != 0) {
            bad_set++;
        }
    }
    CHECK(bad_move == 0, "memmove disagreed with libc %d times", bad_move);
    CHECK(bad_cmp == 0, "memcmp sign disagreed with libc %d times", bad_cmp);
    CHECK(bad_set == 0, "memset disagreed with libc %d times", bad_set);

    CHECK(arena_strlen("") == 0 && arena_strlen("hello") == 5, "strlen");
    CHECK(arena_strcmp("abc", "abc") == 0 && arena_strcmp("abc", "abd") < 0 &&
              arena_strcmp("b", "abc") > 0,
          "strcmp");
    CHECK(arena_strncmp("abcX", "abcY", 3) == 0 && arena_strncmp("abcX", "abcY", 4) < 0,
          "strncmp");
    CHECK(arena_strnlen("abcdef", 3) == 3 && arena_strnlen("ab", 9) == 2, "strnlen");
    /* overlapping moves in both directions, exact expected values */
    unsigned char m[16];
    for (unsigned i = 0; i < 16; i++) {
        m[i] = (unsigned char)i;
    }
    arena_memmove(m + 2, m, 8);
    CHECK(m[2] == 0 && m[9] == 7 && m[10] == 10, "backward-overlap memmove");
    for (unsigned i = 0; i < 16; i++) {
        m[i] = (unsigned char)i;
    }
    arena_memmove(m, m + 2, 8);
    CHECK(m[0] == 2 && m[7] == 9 && m[8] == 8, "forward-overlap memmove");
}

/* ---- allocator: shadow-model stress ------------------------------------ */

struct live {
    unsigned char *p;
    size_t n;
    unsigned char seed;
};

static int overlaps(const struct live *arr, size_t cnt, const unsigned char *p, size_t n) {
    /* Blocks in the arena never overlap; compare intervals of user payloads. */
    for (size_t i = 0; i < cnt; i++) {
        const unsigned char *q = arr[i].p;
        if (p < q + arr[i].n && q < p + n) {
            return 1;
        }
    }
    return 0;
}

static void test_allocator_stress(void) {
    enum { MAXLIVE = 400, OPS = 200000 };
    struct arena_heap_stats start_stats;
    arena_heap_stats(&start_stats);
    struct live *arr = calloc(MAXLIVE, sizeof *arr);
    size_t cnt = 0;
    uint64_t refused_seen = 0, huge_refused = 0, corrupt = 0, overlap = 0;
    uint64_t allocs_ok = 0, frees = 0;

    for (int op = 0; op < OPS; op++) {
        unsigned r = (unsigned)(rnd() % 100);
        if (r < 45 || cnt == 0) {
            if (cnt == MAXLIVE) {
                continue;
            }
            size_t n;
            unsigned k = (unsigned)(rnd() % 100);
            if (k < 70) {
                n = 1 + rnd() % 256;
            } else if (k < 95) {
                n = 256 + rnd() % 4000;
            } else if (k < 99) {
                n = 4096 + rnd() % (256 * 1024);
            } else {
                n = (1u << 20) + 1 + rnd() % 4096; /* above the 1 MiB ceiling */
            }
            unsigned char *p = arena_malloc(n);
            if (p == NULL) {
                refused_seen++;
                if (n > (1u << 20)) {
                    huge_refused++;
                }
                continue;
            }
            allocs_ok++;
            if (overlaps(arr, cnt, p, n)) {
                overlap++;
            }
            unsigned char seed = (unsigned char)rnd();
            for (size_t j = 0; j < n; j++) {
                p[j] = (unsigned char)(seed + j);
            }
            arr[cnt++] = (struct live){p, n, seed};
        } else if (r < 70) {
            size_t idx = rnd() % cnt;
            struct live e = arr[idx];
            for (size_t j = 0; j < e.n; j++) {
                if (e.p[j] != (unsigned char)(e.seed + j)) {
                    corrupt++;
                    break;
                }
            }
            arena_free(e.p);
            frees++;
            arr[idx] = arr[--cnt];
        } else if (r < 85) {
            size_t idx = rnd() % cnt;
            struct live *e = &arr[idx];
            size_t want = 1 + rnd() % 8192;
            unsigned char *q = arena_realloc(e->p, want);
            if (q == NULL) {
                refused_seen++; /* original must still be intact */
                for (size_t j = 0; j < e->n; j++) {
                    if (e->p[j] != (unsigned char)(e->seed + j)) {
                        corrupt++;
                        break;
                    }
                }
                continue;
            }
            size_t keep = e->n < want ? e->n : want;
            for (size_t j = 0; j < keep; j++) {
                if (q[j] != (unsigned char)(e->seed + j)) {
                    corrupt++;
                    break;
                }
            }
            e->p = q;
            e->n = want;
            for (size_t j = 0; j < want; j++) {
                q[j] = (unsigned char)(e->seed + j);
            }
        } else {
            size_t idx = rnd() % cnt;
            struct live *e = &arr[idx];
            size_t count = 1 + rnd() % 64, size = 1 + rnd() % 64;
            unsigned char *z = arena_calloc(count, size);
            if (z == NULL) {
                refused_seen++;
                continue;
            }
            for (size_t j = 0; j < count * size; j++) {
                if (z[j] != 0) {
                    corrupt++;
                    break;
                }
            }
            arena_free(z); /* zeroed and freed immediately: no live entry */
            frees++;
            (void)e;
        }
    }

    struct arena_heap_stats mid;
    arena_heap_stats(&mid);
    CHECK(corrupt == 0, "%" PRIu64 " payload corruptions", corrupt);
    CHECK(overlap == 0, "%" PRIu64 " overlapping live allocations", overlap);
    CHECK(mid.live_blocks == cnt, "live_blocks %" PRIu64 " != shadow %zu", mid.live_blocks, cnt);
    CHECK(mid.refused >= huge_refused, "refused counter below observed refusals");
    CHECK(huge_refused > 0, "stress never exercised the over-1MiB refusal path");
    CHECK(mid.reuse_hits > 0, "free lists were never reused");

    for (size_t i = 0; i < cnt; i++) {
        arena_free(arr[i].p);
    }
    struct arena_heap_stats end;
    arena_heap_stats(&end);
    CHECK(end.live_blocks == 0 && end.live_bytes == 0, "live state did not return to zero");
    CHECK(end.bad_frees == start_stats.bad_frees,
          "well-formed stress produced %" PRIu64 " bad frees",
          end.bad_frees - start_stats.bad_frees);
    CHECK(end.committed_pages <= end.reserved_pages, "committed beyond reservation");
    fprintf(stdout,
            "  stress: %d ops, %" PRIu64 " allocs ok, %" PRIu64 " frees, %" PRIu64
            " refusals (%" PRIu64 " over-ceiling), reuse=%" PRIu64 ", committed=%" PRIu64
            "/%" PRIu64 " pages\n",
            OPS, allocs_ok, frees, refused_seen, huge_refused, end.reuse_hits,
            end.committed_pages, end.reserved_pages);
    free(arr);
}

static void test_allocator_contract(void) {
    /* Zero-size, alignment, realloc semantics, and bad-free accounting. */
    void *z = arena_malloc(0);
    CHECK(z != NULL && ((uintptr_t)z % 16) == 0, "malloc(0) must return a 16-aligned block");
    arena_free(z);
    for (size_t n = 1; n <= 1024; n *= 2) {
        void *p = arena_malloc(n);
        CHECK(p != NULL && ((uintptr_t)p % 16) == 0, "alignment for n=%zu", n);
        arena_free(p);
    }
    unsigned char *p = arena_malloc(40);
    memset(p, 0x5A, 40);
    unsigned char *q = arena_realloc(p, 40);
    CHECK(q == p, "realloc within capacity must not move");
    unsigned char *r = arena_realloc(p, 2000);
    CHECK(r != NULL && r[0] == 0x5A && r[39] == 0x5A, "realloc growth must preserve bytes");
    struct arena_heap_stats before, after;
    arena_heap_stats(&before);
    arena_free(r);
    arena_free(r);          /* double free */
    arena_free(r + 16);     /* interior pointer */
    arena_free((void *)1);  /* wild pointer */
    arena_heap_stats(&after);
    CHECK(after.bad_frees == before.bad_frees + 3, "three bad frees must be counted");
    CHECK(after.live_blocks == before.live_blocks - 1, "only the valid free may change live state");
    void *zz = arena_realloc(NULL, 33);
    CHECK(zz != NULL, "realloc(NULL, n) acts as malloc");
    CHECK(arena_realloc(zz, 0) == NULL, "realloc(p, 0) frees and returns NULL");
    CHECK(arena_calloc((size_t)-1, 2) == NULL, "calloc overflow must be refused");
    arena_free(NULL);
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    fprintf(stdout, "HOST-ONLY arena C runtime tests (ASan+UBSan, glibc differential)\n");
    test_format_fixed();
    test_format_differential();
    test_memory_string();
    test_allocator_contract();
    test_allocator_stress();
    if (failures == 0) {
        fprintf(stdout, "HOST-ONLY RESULT PASS (%d checks)\n", checks);
        return 0;
    }
    fprintf(stdout, "HOST-ONLY RESULT FAIL (%d of %d checks)\n", failures, checks);
    return 1;
}
