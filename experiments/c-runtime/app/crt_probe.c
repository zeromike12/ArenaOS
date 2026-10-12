/*
 * crt_probe - the minimal genuine freestanding C application that runs in
 * ArenaOS guest (experimental; docs/compat/03-c-runtime-prototype.md).
 *
 * Every group checks its result against an expectation computed WITHOUT the
 * runtime under test (literal strings, independent loops, measured clock
 * bounds). Negative controls are included: an oversized allocation must be
 * refused, and deliberately bad frees must be counted and ignored.
 *
 * Exit status: 37 when all groups pass (matches the repo's startup-proof
 * convention), 2 otherwise.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/rt.h"
#include "arena/string.h"

#define GROUPS 6
#define PASS_EXIT 37
#define FAIL_EXIT 2

static int passed;

static void report(const char *name, int ok) {
    arena_printf("crt-probe: %s %s\n", name, ok ? "PASS" : "FAIL");
    if (ok) {
        passed++;
    }
}

/* ---- T1: entry, stack switch, TLS installed ---------------------------- */
static int t1_entry_stack(void) {
    uintptr_t lo = 0, hi = 0;
    arena_stack_bounds(&lo, &hi);
    volatile int marker = 0;
    uintptr_t sp = (uintptr_t)&marker;
    arena_printf("crt-probe: T1 stack [0x%lx,0x%lx) size=%lu sp=0x%lx tls=%d\n",
                 (unsigned long)lo, (unsigned long)hi,
                 (unsigned long)(hi - lo), (unsigned long)sp, arena_tls_ready());
    return lo != 0 && hi - lo == 256u * 1024u && sp >= lo && sp < hi &&
           arena_tls_ready() == 1;
}

/* ---- T2: TLS: .tdata initial value, .tbss zero, per-thread storage ----- */
static __thread uint64_t tls_init_val = 0x5A5Aull;
static __thread uint64_t tls_zero;

static int t2_tls(void) {
    int ok = tls_init_val == 0x5A5Aull && tls_zero == 0;
    tls_init_val += 1;
    tls_zero += 2;
    ok = ok && tls_init_val == 0x5A5Bull && tls_zero == 2;
    arena_printf("crt-probe: T2 tls init=0x%lx zero=%lu\n",
                 (unsigned long)tls_init_val, (unsigned long)tls_zero);
    return ok;
}

/* ---- T3: formatted output, exact text ----------------------------------- */
static int t3_format(void) {
    char buf[160];
    int n = arena_snprintf(buf, sizeof buf,
                           "%d|%5u|%-6d|%06d|%x|%X|%llx|%ld|%s|%.3s|%c|%%|%-4s|%hhd",
                           -42, 7u, 12, -3, 255u, 0xBEEFu, 0x123456789ABCDEF0ull,
                           -9L, "str", "abcdef", 'Z', "ab", (int)300);
    const char *want = "-42|    7|12    |-00003|ff|BEEF|123456789abcdef0|-9|str|abc|Z|%|ab  |44";
    int ok = n == (int)arena_strlen(want) && arena_strcmp(buf, want) == 0;

    /* Truncation: capacity 8 keeps 7 chars + NUL, but reports full length. */
    char small[8];
    int m = arena_snprintf(small, sizeof small, "%s", "0123456789");
    ok = ok && m == 10 && arena_strcmp(small, "0123456") == 0;

    arena_printf("crt-probe: T3 formatted=\"%s\" len=%d\n", buf, n);
    return ok;
}

/* ---- T4: memory --------------------------------------------------------- */
#define NBLOCKS 384
static uint8_t *blocks[NBLOCKS];
static size_t sizes[NBLOCKS];

static size_t size_for(unsigned i) {
    return 1u + (size_t)((i * 37u) % 3000u);
}

static void fill(uint8_t *p, size_t n, unsigned seed) {
    for (size_t j = 0; j < n; j++) {
        p[j] = (uint8_t)((seed + j * 13u) & 0xFF);
    }
}

static int check(const uint8_t *p, size_t n, unsigned seed) {
    for (size_t j = 0; j < n; j++) {
        if (p[j] != (uint8_t)((seed + j * 13u) & 0xFF)) {
            return 0;
        }
    }
    return 1;
}

static int t4_memory(void) {
    /* crt0's TLS block is a live allocation made before main(); measure the
     * heap counters relative to that baseline, not against zero. */
    struct arena_heap_stats pre;
    arena_heap_stats(&pre);
    int ok = 1;
    for (unsigned i = 0; i < NBLOCKS; i++) {
        sizes[i] = size_for(i);
        blocks[i] = arena_malloc(sizes[i]);
        if (blocks[i] == NULL) {
            ok = 0;
            break;
        }
        fill(blocks[i], sizes[i], i);
    }
    for (unsigned i = 0; ok && i < NBLOCKS; i++) {
        ok = check(blocks[i], sizes[i], i);
    }
    /* Free the even blocks, reuse the classes, and prove odd survivors are
     * intact (no overlap between live blocks and reused space). */
    for (unsigned i = 0; i < NBLOCKS; i += 2) {
        arena_free(blocks[i]);
        blocks[i] = NULL;
    }
    for (unsigned i = 0; ok && i < NBLOCKS; i += 2) {
        blocks[i] = arena_malloc(sizes[i]);
        ok = blocks[i] != NULL;
        if (ok) {
            fill(blocks[i], sizes[i], i + 1000u);
        }
    }
    for (unsigned i = 0; ok && i < NBLOCKS; i++) {
        ok = check(blocks[i], sizes[i], i % 2 == 0 ? i + 1000u : i);
    }

    /* calloc must return zeroed memory even when it reuses a dirty block. */
    uint8_t *dirty = arena_malloc(96);
    ok = ok && dirty != NULL;
    if (ok) {
        arena_memset(dirty, 0xCC, 96);
        arena_free(dirty);
        uint8_t *z = arena_calloc(12, 8);
        ok = z != NULL && z == dirty;
        for (unsigned j = 0; ok && j < 96; j++) {
            ok = z[j] == 0;
        }
        arena_free(z);
    }

    /* realloc growth preserves the prefix. */
    uint8_t *g = arena_malloc(100);
    ok = ok && g != NULL;
    if (ok) {
        fill(g, 100, 77u);
        uint8_t *g2 = arena_realloc(g, 5000);
        ok = g2 != NULL && check(g2, 100, 77u);
        arena_free(g2);
    }

    /* One large block (256 KiB) across several committed chunks. */
    uint8_t *big = arena_malloc(256u * 1024u);
    ok = ok && big != NULL;
    if (ok) {
        fill(big, 256u * 1024u, 9u);
        ok = check(big, 256u * 1024u, 9u);
        arena_free(big);
    }

    /* Negative control: above the allocator ceiling must be refused. The C1
     * allocator (HEAP_PAGES 4096 = 16 MiB) replaced the prototype's 4 MiB
     * heap; the expectation follows that contract, the check is unchanged. */
    ok = ok && arena_malloc(17u * 1024u * 1024u) == NULL;

    /* Exhaust the bounded heap; refusal must be a NULL return, not a fault. */
    /* Bounded by 512 x 64 KiB = 32 MiB, larger than the 16 MiB heap, so the
     * loop always reaches refusal regardless of the heap size. */
    static uint8_t *hog[512];
    unsigned hogs = 0;
    while (hogs < 512) {
        hog[hogs] = arena_malloc(64u * 1024u);
        if (hog[hogs] == NULL) {
            break;
        }
        hogs++;
    }
    ok = ok && hogs > 0 && hogs < 512;

    /* Negative control: bad frees are counted and ignored. */
    uint8_t *victim = arena_malloc(64);
    ok = ok && victim != NULL;
    if (ok) {
        arena_free(victim);
        arena_free(victim);       /* double free */
        arena_free(victim + 8);   /* interior pointer */
    }

    /* After exhaustion, freed memory is reusable again. */
    for (unsigned h = 0; h < hogs; h++) {
        arena_free(hog[h]);
    }
    uint8_t *again = arena_malloc(64u * 1024u);
    ok = ok && again != NULL;
    arena_free(again);

    /* Release everything still live so the counters close exactly. */
    for (unsigned i = 0; i < NBLOCKS; i++) {
        arena_free(blocks[i]);
        blocks[i] = NULL;
    }

    struct arena_heap_stats st;
    arena_heap_stats(&st);
    ok = ok && st.live_blocks == pre.live_blocks && st.live_bytes == pre.live_bytes &&
         st.bad_frees == pre.bad_frees + 2 && st.reuse_hits > pre.reuse_hits &&
         st.refused >= pre.refused + 2 && st.committed_pages >= 16;
    arena_printf("crt-probe: T4 heap reserved=%lu committed=%lu live=%lu (pre %lu) "
                 "reuse=%lu refused=%lu bad_frees=%lu hogs=%u\n",
                 (unsigned long)st.reserved_pages, (unsigned long)st.committed_pages,
                 (unsigned long)st.live_blocks, (unsigned long)pre.live_blocks,
                 (unsigned long)st.reuse_hits,
                 (unsigned long)st.refused, (unsigned long)st.bad_frees, hogs);
    return ok;
}

/* ---- T5: string and memory primitives, independent expectations -------- */
static int t5_strings(void) {
    uint8_t a[1000], b[1000];
    for (unsigned i = 0; i < sizeof a; i++) {
        a[i] = (uint8_t)(i * 7u + 3u);
    }
    arena_memcpy(b, a, sizeof a);
    int ok = arena_memcmp(a, b, sizeof a) == 0;

    /* Forward overlap (dst < src) and backward overlap (dst > src). */
    uint8_t m[64];
    for (unsigned i = 0; i < 64; i++) {
        m[i] = (uint8_t)i;
    }
    arena_memmove(m + 0, m + 8, 32);   /* expect m[0..31] = old m[8..39] */
    ok = ok && m[0] == 8 && m[31] == 39 && m[32] == 32;
    for (unsigned i = 0; i < 64; i++) {
        m[i] = (uint8_t)i;
    }
    arena_memmove(m + 8, m + 0, 32);   /* expect m[8..39] = old m[0..31] */
    ok = ok && m[8] == 0 && m[39] == 31 && m[40] == 40;

    uint8_t s[100];
    arena_memset(s, 0x7F, sizeof s);
    ok = ok && s[0] == 0x7F && s[99] == 0x7F;

    ok = ok && arena_memcmp("abc", "abd", 3) < 0 && arena_memcmp("abd", "abc", 3) > 0;
    ok = ok && arena_strlen("hello") == 5 && arena_strlen("") == 0;
    ok = ok && arena_strcmp("abc", "abc") == 0 && arena_strcmp("abc", "abd") < 0;
    ok = ok && arena_strncmp("abcX", "abcY", 3) == 0 && arena_strncmp("abcX", "abcY", 4) < 0;
    arena_printf("crt-probe: T5 memcpy/memmove/memset/memcmp/strlen/strcmp verified\n");
    return ok;
}

/* ---- T6: time ----------------------------------------------------------- */
static int t6_time(void) {
    uint64_t prev = arena_clock_us();
    uint64_t start = prev;
    unsigned violations = 0;
    for (unsigned i = 0; i < 2000; i++) {
        arena_yield();
        uint64_t now = arena_clock_us();
        if (now < prev) {
            violations++;
        }
        prev = now;
    }
    uint64_t t0 = arena_clock_us();
    arena_busy_sleep_us(20000);
    uint64_t elapsed = arena_clock_us() - t0;
    arena_printf("crt-probe: T6 clock start=%lu us, yield-loop=%lu us, "
                 "sleep(20000)=%lu us, monotonic_violations=%u\n",
                 (unsigned long)start, (unsigned long)(prev - start),
                 (unsigned long)elapsed, violations);
    return violations == 0 && elapsed >= 20000u && elapsed < 5000000u;
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    /* Legacy scratch probe: it runs without a granted stream set and reports
     * over the explicit SYS_DEBUG_WRITE opt-in. Ordinary C applications never
     * call this. */
    arena_legacy_serial_enable();
    arena_printf("crt-probe: start (freestanding C, ArenaOS prototype runtime)\n");
    report("T1 entry-stack-tls", t1_entry_stack());
    report("T2 tls", t2_tls());
    report("T3 format", t3_format());
    report("T4 memory", t4_memory());
    report("T5 strings", t5_strings());
    report("T6 time", t6_time());
    int all = passed == GROUPS;
    arena_printf("CRT-PROBE RESULT %s (%d/%d)\n", all ? "PASS" : "FAIL", passed, GROUPS);
    return all ? PASS_EXIT : FAIL_EXIT;
}
