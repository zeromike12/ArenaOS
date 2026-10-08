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
#include <sys/wait.h>
#include <unistd.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "arena/abi.h"
#include "arena/rt.h"
#include "arena/string.h"

static int checks;
static int failures;
static int known_defect_reproduced;

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
                n = (16u << 20) + 1 + rnd() % 4096; /* above the 16 MiB ceiling */
            }
            unsigned char *p = arena_malloc(n);
            if (p == NULL) {
                refused_seen++;
                if (n > (16u << 20)) {
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
    CHECK(huge_refused > 0, "stress never exercised the over-16MiB refusal path");
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

/* ---- allocator: hardening, bounded coalescing, ceiling ------------------ */

static void test_allocator_hardening(void) {
    struct arena_heap_stats s0, s1;
    arena_heap_stats(&s0);

    /* Interior and freed pointers are rejected before any metadata is read. */
    unsigned char *p = arena_malloc(64);
    CHECK(p != NULL, "hardening: base allocation");
    memset(p, 0x3C, 64);
    arena_free(p + 1);
    arena_free(p + 40);
    arena_heap_stats(&s1);
    CHECK(s1.bad_frees == s0.bad_frees + 2, "interior frees must be counted as bad");
    CHECK(p[0] == 0x3C && p[63] == 0x3C, "rejected interior free must not disturb the block");
    arena_free(p);
    arena_heap_stats(&s1);
    CHECK(s1.bad_frees == s0.bad_frees + 2, "a valid free must not count as bad");
    arena_free(p);
    arena_heap_stats(&s1);
    CHECK(s1.bad_frees == s0.bad_frees + 3, "freed-pointer reuse must be rejected");

    /* realloc of a freed pointer must not trust it: NULL, counted, no crash. */
    void *stale = arena_malloc(96);
    arena_free(stale);
    void *again = arena_realloc(stale, 200);
    CHECK(again == NULL, "realloc of a freed pointer must return NULL");
    arena_heap_stats(&s1);
    CHECK(s1.bad_frees == s0.bad_frees + 4, "realloc of a freed pointer must count as bad");

    /* Ceiling: one byte past the 16 MiB payload is refused, and refusal is
     * counted without corrupting the heap. */
    size_t over = (size_t)16u << 20;
    CHECK(arena_malloc(over) == NULL, "16 MiB request must be refused");
    void *fit = arena_malloc((size_t)8u << 20);
    CHECK(fit != NULL, "8 MiB request must succeed");
    arena_free(fit);

    /* Bounded coalescing: free many small blocks in shuffled order, then a
     * large contiguous block must be available again. */
    enum { N = 512 };
    static void *blocks[N];
    for (int i = 0; i < N; i++) {
        blocks[i] = arena_malloc(4096);
        CHECK(blocks[i] != NULL, "coalescing: 4 KiB allocation %d", i);
    }
    for (int i = N - 1; i > 0; i--) {
        int j = (int)(rnd() % (uint64_t)(i + 1));
        void *t = blocks[i];
        blocks[i] = blocks[j];
        blocks[j] = t;
    }
    for (int i = 0; i < N; i++) {
        arena_free(blocks[i]);
    }
    void *big = arena_malloc((size_t)12u << 20);
    CHECK(big != NULL, "freed small blocks must coalesce into a 12 MiB block");
    arena_free(big);
    arena_heap_stats(&s1);
    CHECK(s1.live_blocks == s0.live_blocks, "hardening: live blocks return to baseline");
}

/* ---- ring protocol: u32 counter wrap ----------------------------------- */

static uint32_t rd_u32(const unsigned char *at) {
    uint32_t v;
    memcpy(&v, at, 4);
    return v;
}

static void wr_u32(unsigned char *at, uint32_t v) {
    memcpy(at, &v, 4);
}

/* Stream ring channel 1 lives at page + 64 + 832; its write counter is at +0,
 * read counter at +4, state at +8, data at +64 (wire layout, streams.c). */
static void test_ring_wrap(void) {
    unsigned char *page = aligned_alloc(4096, 4096);
    CHECK(page != NULL, "ring: page allocation");
    memset(page, 0, 4096);
    CHECK(arena_ring_page_init(page) == 0, "ring: page init");
    unsigned char *ring = page + 64 + 832;

    /* Positive case: 100 bytes, then 200 bytes whose counter crosses 2^32
     * (0xFFFFFF64 + 200 wraps to 0x2C). Occupancy stays far below capacity, so
     * the transfer round-trips byte-exact across the wrap. */
    wr_u32(ring + 0, 0xFFFFFF00u);
    wr_u32(ring + 4, 0xFFFFFF00u);
    unsigned char src[300], dst[300];
    for (unsigned i = 0; i < sizeof src; i++) {
        src[i] = (unsigned char)(i * 7u + 3u);
    }
    CHECK(arena_ring_write_some(page, 1, src, 100) == 100, "ring wrap: pre-wrap write");
    CHECK(arena_ring_read_some(page, 1, dst, 100) == 100, "ring wrap: pre-wrap read");
    CHECK(arena_ring_write_some(page, 1, src + 100, 200) == 200, "ring wrap: write across 2^32");
    CHECK(rd_u32(ring + 0) == 0x0000002Cu, "ring wrap: write counter wrapped to 0x2C");
    CHECK(arena_ring_read_some(page, 1, dst + 100, 200) == 200, "ring wrap: read across 2^32");
    CHECK(memcmp(dst, src, sizeof src) == 0, "ring wrap: byte order across 2^32");

    /* Known protocol defect, characterization pin (NOT a pass of the property):
     * with head and tail at 0xFFFFFF00, a 256-byte write followed by a 512-byte
     * write straddles the u32 wrap. Both writes use index = counter % 768, and
     * 2^32 % 768 == 256, so the second write lands on residues still holding
     * unread bytes of the first. The corruption is reproduced here and reported
     * as a protocol finding. A fix changes the wire format and needs an ADR. */
    wr_u32(ring + 0, 0xFFFFFF00u);
    wr_u32(ring + 4, 0xFFFFFF00u);
    unsigned char a[256], b[512];
    memset(a, 0xAA, sizeof a);
    memset(b, 0xBB, sizeof b);
    CHECK(arena_ring_write_some(page, 1, a, sizeof a) == 256, "defect pin: first write");
    CHECK(arena_ring_write_some(page, 1, b, sizeof b) == 512, "defect pin: second write");
    unsigned char out[768];
    CHECK(arena_ring_read_some(page, 1, out, sizeof out) == 768, "defect pin: drain");
    int overwritten = memcmp(out, a, 256) != 0;
    if (overwritten) {
        printf("[host] KNOWN-DEFECT ring u32-wrap discontinuity reproduced: "
               "unread bytes overwritten (protocol ADR required)\n");
        known_defect_reproduced = 1;
    } else {
        CHECK(0, "defect pin: expected overwrite did not reproduce; re-review the ring protocol");
    }
    free(page);
}

/* Host-only VM fault injection (vm.c, ARENA_HOSTED). Not guest evidence. */
void arena_vm_host_fail_reserve_from(int64_t n);
void arena_vm_host_fail_commit_from(int64_t n);
void arena_vm_host_reset(void);

/* Commit refusal in a running process: a request that needs uncommitted
 * space must be refused, and nothing already live may change. */
/* C1.1 partial transfers and peer failure on the printf path. The host
 * arena_write seam can cap each write (short writes) or fail every write. */
extern long arena_host_write_limit;
extern long arena_host_write_fail;
extern int arena_host_capture_on;
extern char arena_host_capture[512];
extern size_t arena_host_capture_n;
extern long arena_host_write_calls;

static void test_stdio_partial_writes(void) {
    static const char want[] = "partial-write-check 0123456789 abcdefghijklmnopqrstuvwxyz 42\n";
    size_t want_n = sizeof want - 1;
    arena_host_capture_on = 1;
    arena_host_capture_n = 0;
    arena_host_write_limit = 3;
    long calls0 = arena_host_write_calls;
    int r = arena_printf("partial-write-check 0123456789 abcdefghijklmnopqrstuvwxyz %d\n", 42);
    long calls = arena_host_write_calls - calls0;
    CHECK(r == (int)want_n, "partial writes: printf reports every byte (got %d, want %zu)", r, want_n);
    CHECK(arena_host_capture_n == want_n && memcmp(arena_host_capture, want, want_n) == 0,
          "partial writes: every byte arrives once, in order, across 3-byte short writes");
    CHECK(calls == (long)((want_n + 2) / 3), "partial writes: the flush retries the remainder (calls=%ld)", calls);

    /* Longer than the 128-byte sink buffer: the buffer-full flush also loops. */
    char longs[200];
    memset(longs, 'Q', sizeof longs - 1);
    longs[sizeof longs - 1] = '\0';
    arena_host_capture_n = 0;
    calls0 = arena_host_write_calls;
    r = arena_printf("%s", longs);
    CHECK(r == (int)(sizeof longs - 1), "partial writes: >128-byte output reports every byte (got %d)", r);
    CHECK(arena_host_capture_n == sizeof longs - 1 && memcmp(arena_host_capture, longs, sizeof longs - 1) == 0,
          "partial writes: >128-byte output arrives intact across the buffer-full flush");
    arena_host_write_limit = 0;
    arena_host_capture_on = 0;
}

static void test_stdio_write_error(void) {
    arena_host_write_fail = ARENA_E_BROKEN_PIPE;
    long calls0 = arena_host_write_calls;
    int r = arena_printf("never-written\n");
    CHECK(r == ARENA_E_BROKEN_PIPE, "write error: printf returns the peer error, not a byte count (got %d)", r);
    CHECK(arena_host_write_calls - calls0 == 1, "write error: one attempt, no retry (calls=%ld)", arena_host_write_calls - calls0);

    /* Multi-flush output: after the first failure, later flushes are skipped. */
    char longs[300];
    memset(longs, 'Z', sizeof longs - 1);
    longs[sizeof longs - 1] = '\0';
    calls0 = arena_host_write_calls;
    r = arena_printf("%s", longs);
    CHECK(r == ARENA_E_BROKEN_PIPE, "write error: multi-flush output returns the first error (got %d)", r);
    CHECK(arena_host_write_calls - calls0 == 1, "write error: later flushes are not attempted after a failure (calls=%ld)",
          arena_host_write_calls - calls0);
    arena_host_write_fail = 0;
}

/* C1.1: a process without a stream grant must fail stdio setup cleanly with
 * ARENA_E_NO_STREAMS and never touch a stream ring. The host startup stub
 * returns no record at all, which is the "no grant" case. */
static void test_stdio_missing_grant(void) {
    int rc = arena_stdio_init();
    CHECK(rc == ARENA_E_NO_STREAMS, "missing stream grant: stdio_init returns ARENA_E_NO_STREAMS (got %d)", rc);
    long w = arena_stream_write_some(1, "x", 1);
    CHECK(w < 0, "missing stream grant: stdout write is refused, not silently accepted (got %ld)", w);
    long r = arena_stream_read_some(0, (char[1]){0}, 1);
    CHECK(r < 0, "missing stream grant: stdin read is refused (got %ld)", r);
}

static void test_vm_commit_refusal(void) {
    struct arena_heap_stats b0, end;
    arena_heap_stats(&b0);
    enum { KEEP = 40, EXTRA = 64 };
    uint8_t *keep[KEEP];
    size_t kn[KEEP];
    int kept = 0;
    for (unsigned i = 0; i < KEEP; i++) {
        kn[i] = 900u + i * 7u;
        keep[i] = arena_malloc(kn[i]);
        if (keep[i] == NULL) {
            break;
        }
        memset(keep[i], (int)(i * 3u + 1u), kn[i]);
        kept++;
    }
    CHECK(kept == KEEP, "commit refusal: setup blocks allocated before arming");

    arena_vm_host_fail_commit_from(0);
    uint8_t *extra[EXTRA];
    int nextra = 0, big_refused = 0, big_total = 0;
    for (unsigned k = 0; k < EXTRA; k++) {
        if (k % 4 == 0) {
            /* 12 MiB needs a top-level block, impossible while small blocks
             * are live, so it must be refused without touching any list. */
            big_total++;
            uint8_t *p = arena_malloc((size_t)12u << 20);
            if (p == NULL) {
                big_refused++;
            } else {
                CHECK(0, "commit refusal: 12 MiB unexpectedly granted with live blocks");
                arena_free(p);
            }
        } else {
            uint8_t *p = arena_malloc(300u + k);
            if (p != NULL) {
                memset(p, 0x5A, 300u + k);
                extra[nextra++] = p;
            }
        }
    }
    CHECK(big_refused == big_total, "commit refusal: every oversize request refused");

    int intact = 1;
    for (unsigned i = 0; i < KEEP && intact; i++) {
        for (size_t j = 0; j < kn[i]; j++) {
            if (keep[i][j] != (uint8_t)(i * 3u + 1u)) {
                intact = 0;
                break;
            }
        }
    }
    CHECK(intact, "commit refusal: live blocks unchanged after refused commits");

    arena_heap_stats(&end);
    CHECK(end.refused >= b0.refused + (uint64_t)big_refused,
          "commit refusal: refusals counted");

    for (int i = 0; i < nextra; i++) {
        arena_free(extra[i]);
    }
    for (unsigned i = 0; i < KEEP; i++) {
        arena_free(keep[i]);
    }
    arena_vm_host_reset();
    arena_heap_stats(&end);
    CHECK(end.live_blocks == b0.live_blocks && end.live_bytes == b0.live_bytes,
          "commit refusal: live counters return to baseline");
    CHECK(end.bad_frees == b0.bad_frees, "commit refusal: no spurious bad frees");

    /* Recovery: with commits restored, the freed space coalesces back into a
     * block large enough for the 12 MiB request. */
    uint8_t *big = arena_malloc((size_t)12u << 20);
    CHECK(big != NULL, "commit recovery: 12 MiB granted after the heap is empty again");
    if (big != NULL) {
        big[0] = 1;
        big[(size_t)12u * 1024u * 1024u - 1u] = 2;
        CHECK(big[0] == 1 && big[(size_t)12u * 1024u * 1024u - 1u] == 2,
              "commit recovery: large block usable");
        arena_free(big);
    }
}

/* Reservation refusal before the heap exists. Needs a fresh process: this
 * test re-executes the binary with "reserve-refusal" so the heap is never
 * initialised by an earlier test. */
static void reserve_refusal_mode(void) {
    arena_vm_host_fail_reserve_from(0);
    void *p = arena_malloc(100);
    struct arena_heap_stats st;
    arena_heap_stats(&st);
    int ok = p == NULL && st.live_blocks == 0;
    void *q = arena_malloc(100); /* the retry path must also refuse cleanly */
    ok = ok && q == NULL;
    arena_free(p);
    arena_free(q);
    fprintf(stdout, "[host] reserve-refusal child: %s\n", ok ? "refused cleanly" : "FAILED");
    exit(ok ? 0 : 1);
}

static void test_vm_reserve_refusal(void) {
    fflush(stdout);
    pid_t pid = fork();
    if (pid == 0) {
        execl("/proc/self/exe", "host-test", "reserve-refusal", (char *)NULL);
        _exit(99);
    }
    int status = 0;
    CHECK(pid > 0 && waitpid(pid, &status, 0) == pid, "reserve refusal: child waited");
    CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 0,
          "reserve refusal: fresh process refused the reservation cleanly");
}

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "reserve-refusal") == 0) {
        reserve_refusal_mode();
    }
    (void)argc;
    (void)argv;
    fprintf(stdout, "HOST-ONLY arena C runtime tests (ASan+UBSan, glibc differential)\n");
    test_format_fixed();
    test_format_differential();
    test_memory_string();
    test_allocator_contract();
    test_allocator_stress();
    test_allocator_hardening();
    test_stdio_missing_grant();
    test_stdio_partial_writes();
    test_stdio_write_error();
    test_vm_commit_refusal();
    test_vm_reserve_refusal();
    test_ring_wrap();
    if (failures == 0 && known_defect_reproduced) {
        fprintf(stdout, "HOST-ONLY RESULT PASS (%d checks, known protocol defect pinned)\n", checks);
        return 0;
    }
    fprintf(stdout, "HOST-ONLY RESULT FAIL (%d of %d checks)\n", failures, checks);
    return 1;
}
