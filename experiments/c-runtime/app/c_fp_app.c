/*
 * Phase 16 FP/SSE2 preservation app (EXPERIMENTAL).
 *
 * Deliberately outside the integer-only build: this application is compiled
 * with the ADR-0113 baseline ring-3 profile (-march=x86-64 -msse2, AVX
 * disabled) by build_fp_app.py and proves the Phase 15 contract from C:
 *
 *   F1  SSE2 double arithmetic in main gives exact IEEE-754 results.
 *   F2  Two C11 worker threads hold *different* live double values across
 *       many thrd_yield() points; each must observe only its own sequence
 *       (per-thread FXSAVE64 state, ADR-0113).
 *   F3  x87 long double multiply in main after the workers join (state not
 *       polluted by worker execution).
 *   F4  Integer printf path still exact; FP conversions keep refusing with
 *       ENOTSUP (the runtime formatter is unchanged; FP execution support
 *       does not imply FP formatting).
 *
 * All floating-point checks compare exact bit patterns of exactly
 * representable values (products/sums of small binary fractions); there is
 * no rounding tolerance and no libm. Volatile sinks keep every operation
 * live across yields so the compiler cannot constant-fold the proof away.
 *
 * Exit status: 71 pass, 72 fail, 58 if stdio cannot attach.
 */
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <threads.h>

#include "arena/rt.h"

#define APP_NAME "[c-fp]"
#define EXIT_PASS 71
#define EXIT_FAIL 72
#define EXIT_NO_STREAMS 58
#define WORKER_ROUNDS 64

static int checks;
static int failed_checks;
static volatile double fp_sink;
static volatile long double x87_sink;

#define EXPECT(cond)                                                            \
    do {                                                                        \
        checks++;                                                               \
        if (!(cond)) {                                                          \
            failed_checks++;                                                    \
            printf(APP_NAME " check failed line %d: " #cond "\n", __LINE__);    \
        }                                                                       \
    } while (0)

static uint64_t bits_of(double v) {
    uint64_t u;
    memcpy(&u, &v, sizeof u);
    return u;
}

/* One worker: seed derives from its id; each round advances its own value by
 * an exactly representable step, yields, and re-reads. Any cross-thread
 * FXSAVE mix-up changes the observed sequence and fails the check. */
static int fp_worker(void *arg) {
    int id = (int)(intptr_t)arg;
    volatile double step = id == 1 ? 0.25 : 0.5;
    volatile double acc = id == 1 ? 1.0 : 2.0;
    int ok = 1;
    for (int round = 0; round < WORKER_ROUNDS; round++) {
        double expect = (id == 1 ? 1.0 : 2.0) + (double)round * (id == 1 ? 0.25 : 0.5);
        if (bits_of(acc) != bits_of(expect)) {
            ok = 0;
        }
        double next = acc + step; /* live SSE2 add across the yield */
        fp_sink = next;
        thrd_yield();
        acc = next;
    }
    double want = (id == 1 ? 1.0 : 2.0) + WORKER_ROUNDS * (id == 1 ? 0.25 : 0.5);
    return (ok && bits_of(acc) == bits_of(want)) ? 100 + id : -id;
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    if (arena_stdio_init() != 0) {
        return EXIT_NO_STREAMS;
    }
    printf(APP_NAME " start\n");

    /* F1: SSE2 double arithmetic, exact results. */
    volatile double a = 1.5, b = 2.25;
    double product = a * b;              /* 3.375 exactly */
    double quotient = product / a;       /* 2.25 exactly */
    fp_sink = product + quotient;
    EXPECT(bits_of(product) == bits_of(3.375));
    EXPECT(bits_of(quotient) == bits_of(2.25));
    volatile float fa = 0.5f, fb = 0.25f;
    float fsum = fa + fb;                /* 0.75f exactly (SSE scalar) */
    uint32_t fbits;
    memcpy(&fbits, &fsum, sizeof fbits);
    EXPECT(fbits == 0x3F400000u);

    /* F2: per-thread FP state across cooperative switches. */
    thrd_t t1, t2;
    int r1 = 0, r2 = 0;
    EXPECT(thrd_create(&t1, fp_worker, (void *)(intptr_t)1) == thrd_success);
    EXPECT(thrd_create(&t2, fp_worker, (void *)(intptr_t)2) == thrd_success);
    /* Main also churns its own FP value while the workers run. */
    volatile double mine = 8.0;
    for (int i = 0; i < WORKER_ROUNDS; i++) {
        mine = mine * 1.0 + 0.125;
        thrd_yield();
    }
    EXPECT(bits_of(mine) == bits_of(8.0 + WORKER_ROUNDS * 0.125));
    EXPECT(thrd_join(t1, &r1) == thrd_success && r1 == 101);
    EXPECT(thrd_join(t2, &r2) == thrd_success && r2 == 102);

    /* F3: x87 long double after worker execution (80-bit, exact small values). */
    volatile long double x = 3.0L, y = 0.5L;
    long double xy = x * y;              /* 1.5 exactly */
    x87_sink = xy;
    EXPECT(xy == 1.5L);

    /* F4: integer formatting exact; FP formatting still refused, no output. */
    char buf[64];
    int n = snprintf(buf, sizeof buf, "%d|%x", -7, 48879u);
    EXPECT(n == (int)strlen(buf) && strcmp(buf, "-7|beef") == 0);
    char fbuf[32] = "untouched";
    errno = 0;
    int fn = snprintf(fbuf, sizeof fbuf, "%f", 1.5);
    EXPECT(fn == -1 && errno == ENOTSUP);

    printf(APP_NAME " RESULT %s checks=%d failed=%d\n",
           failed_checks == 0 ? "PASS" : "FAIL", checks, failed_checks);
    return failed_checks == 0 ? EXIT_PASS : EXIT_FAIL;
}
