/*
 * Phase 16 FP/SSE2 integration app (EXPERIMENTAL, corrected).
 *
 * Compiled by build_fp_app.py with the ADR-0113 baseline ring-3 profile.
 * Deliberately separates three claims that a previous version conflated:
 *
 *   F1  High-level C FP *arithmetic correctness* (SSE2 scalar double/float,
 *       x87 long double). The compiler is free to spill these values; this
 *       group proves execution and results, NOT register preservation.
 *   F2  Arithmetic from two C11 threads with thread-owned result slots
 *       (no shared sink; results are read only after thrd_join, which is
 *       the synchronization point). Still not a register claim.
 *   F3  Architectural register preservation via fp_probe.S: xmm0-xmm7 and
 *       three x87 slots hold distinct patterns across (a) no transition
 *       (control), (b) a raw SYS_THREAD_YIELD, and (c) raw SYS_SYNC_WAIT
 *       transitions. Every probe checks BOTH the FXSAVE image
 *       (fp_image_check.h, host-tested) AND the syscall result the native
 *       ABI requires for that transition (yield/control 0, parked timeout
 *       STATUS_TIMEOUT with elapsed >= the deadline, parked wake 0 with a
 *       waker that first observes a parked waiter and reports exactly one
 *       waiter woken, stale sequence immediate 0). A bad-token wait is an
 *       immediate-refusal negative control (STATUS_BAD_ARG, elapsed far
 *       below its deadline): it is evidence about refusal only, never
 *       counted as a blocking transition. The image is captured by
 *       FXSAVE64 immediately after the one raw syscall in fp_probe.S.
 *   F4  Timer-preemption *exposure*: both workers busy-spin in FP code for
 *       ~50 ms (several 10 ms quanta) while runnable, then re-run the F3
 *       probe. Userspace cannot count timer switches directly; the
 *       mechanism itself is Phase 15's m15 kernel evidence. This group is
 *       labeled exposure, not a witnessed-switch proof.
 *   F5  Integer printf exact; FP conversions still refuse (ENOTSUP).
 *
 * Thread creation failure is handled: only created handles are joined.
 * Exit status: 71 pass, 72 fail, 58 if stdio cannot attach. A missing
 * sync grant prints SKIP; the guest judge requires skips=0 for the
 * granted positive/mutant bundles, so a skip is a suite failure there.
 */
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <threads.h>

#include "arena/rt.h"
#include "fp_image_check.h"
#include "sysabi.h" /* internal raw-syscall header: deliberate for this probe app */

#define APP_NAME "[c-fp]"
#define EXIT_PASS 71
#define EXIT_FAIL 72
#define EXIT_NO_STREAMS 58
#define WORKER_ROUNDS 64
#define PROBE_ROUNDS 8

int64_t fp_probe_run(uint64_t nr, uint64_t a0, uint64_t a1, uint64_t a2, uint64_t a3,
                     const void *xmm_vals, void *fxsave_out);

static int checks, failed_checks, skips;
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

/* Distinct 128-byte XMM pattern per owner: byte = owner*37 + index. */
static void make_pattern(uint8_t xmm[128], unsigned owner) {
    for (unsigned i = 0; i < 128; i++) {
        xmm[i] = (uint8_t)(owner * 37u + i);
    }
}

/* One probe transition on this thread. The syscall result is evidence,
 * never discarded: an immediate error (for example BAD_CALL from unused
 * argument registers) must not be mistaken for the intended transition. */
struct probe_result {
    int64_t status;     /* fp_probe_run's returned syscall status */
    int image;          /* fp_image_check verdict: 0 = exact image */
    uint64_t elapsed_us;
};

static struct probe_result probe_once(uint64_t nr, uint64_t a0, uint64_t a1,
                                      uint64_t a2, uint64_t a3, unsigned owner) {
    uint8_t xmm[128];
    _Alignas(16) static _Thread_local uint8_t fx[512];
    make_pattern(xmm, owner);
    uint64_t start = arena_clock_us();
    int64_t status = fp_probe_run(nr, a0, a1, a2, a3, xmm, fx);
    struct probe_result result = {status, fp_image_check(fx, xmm),
                                  arena_clock_us() - start};
    return result;
}

static int probe_ok(struct probe_result p, int64_t expected_status) {
    return p.status == expected_status && p.image == 0;
}

/* Two checks: the exact status this transition must return, and the
 * FXSAVE image captured immediately after it. */
static void expect_probe(struct probe_result p, int64_t expected_status) {
    EXPECT(p.status == expected_status);
    EXPECT(p.image == 0);
    if (p.status != expected_status || p.image != 0) {
        printf(APP_NAME " probe detail status=%lld expected=%lld image_rule=%d elapsed_us=%llu\n",
               (long long)p.status, (long long)expected_status, p.image,
               (unsigned long long)p.elapsed_us);
    }
}

struct worker_result {
    volatile double final_value; /* written only by its owner thread */
    volatile int probe_failures;         /* FXSAVE image mismatches */
    volatile int probe_status_failures;  /* non-zero yield statuses */
    volatile int rounds_done;
};
static struct worker_result results[2];

static int fp_worker(void *arg) {
    int id = (int)(intptr_t)arg; /* 1 or 2 */
    struct worker_result *r = &results[id - 1];
    volatile double step = id == 1 ? 0.25 : 0.5;
    volatile double acc = id == 1 ? 1.0 : 2.0;
    for (int round = 0; round < WORKER_ROUNDS; round++) {
        acc = acc + step;
        thrd_yield();
    }
    r->final_value = acc;
    for (int round = 0; round < PROBE_ROUNDS; round++) {
        struct probe_result p =
            probe_once(ARENA_SYS_THREAD_YIELD, 0, 0, 0, 0, (unsigned)id);
        if (p.image != 0) {
            r->probe_failures++;
        }
        if (p.status != 0) {
            r->probe_status_failures++;
        }
        r->rounds_done++;
    }
    return 100 + id;
}

/* Busy-spin in FP code for at least `us` microseconds of wall time. */
static void spin_fp(uint64_t us) {
    volatile double v = 1.0;
    uint64_t start = arena_clock_us();
    while (arena_clock_us() - start < us) {
        for (int i = 0; i < 256; i++) {
            v = v * 1.0 + 0.5;
        }
    }
    if (bits_of(v) == 0) { /* never true; keeps the loop observable */
        printf(APP_NAME " spin sink\n");
    }
}

static int spin_worker(void *arg) {
    int id = (int)(intptr_t)arg;
    spin_fp(50000);
    return probe_ok(probe_once(ARENA_SYS_THREAD_YIELD, 0, 0, 0, 0,
                                (unsigned)id + 2u), 0)
               ? 120 + id
               : -id;
}

/* F3c wake path: this worker must first OBSERVE the main thread parked
 * (SYS_SYNC_INFO parked_waiters > 0) and only then wake it. Waking before
 * the wait would degenerate into the stale-sequence immediate return, so
 * the observation is part of the proof, recorded in `saw_parked`. */
struct wake_job {
    uint64_t slot;
    uint64_t token;
    volatile int64_t wake_status;
    volatile int saw_parked;
};
static struct wake_job wake_job;

static int wake_worker(void *arg) {
    (void)arg;
    uint64_t words[4];
    uint64_t start = arena_clock_us();
    while (arena_clock_us() - start < 250000u) {
        if (arena_syscall6(ARENA_SYS_SYNC_INFO, wake_job.slot,
                           (uint64_t)(uintptr_t)words, 0, 0, 0, 0) == 0 &&
            words[2] > 0) {
            wake_job.saw_parked = 1;
            break;
        }
        arena_busy_sleep_us(200);
    }
    /* Wake even on observation failure so the main thread is released by
     * its deadline path being the failure, not by hanging here. */
    wake_job.wake_status = arena_syscall6(ARENA_SYS_SYNC_WAKE, wake_job.slot,
                                          wake_job.token, 1, 0, 0, 0);
    return 130;
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    if (arena_stdio_init() != 0) {
        return EXIT_NO_STREAMS;
    }
    printf(APP_NAME " start\n");

    /* F1: arithmetic correctness (no preservation claim). */
    volatile double a = 1.5, b = 2.25;
    EXPECT(bits_of(a * b) == bits_of(3.375));
    EXPECT(bits_of((a * b) / a) == bits_of(2.25));
    volatile float fa = 0.5f, fb = 0.25f;
    float fsum = fa + fb;
    uint32_t fbits;
    memcpy(&fbits, &fsum, sizeof fbits);
    EXPECT(fbits == 0x3F400000u);
    volatile long double x = 3.0L, y = 0.5L;
    EXPECT(x * y == 1.5L);

    /* F3a/F3b in main: control, then yield transitions (each expects the
     * exact status 0 and an exact image). */
    expect_probe(probe_once(0, 0, 0, 0, 0, 9), 0);
    for (int i = 0; i < PROBE_ROUNDS; i++) {
        expect_probe(probe_once(ARENA_SYS_THREAD_YIELD, 0, 0, 0, 0, 9), 0);
    }

    /* F2 (+F3b in workers): arithmetic + probes, results via own slots. */
    thrd_t t1, t2;
    int have1 = thrd_create(&t1, fp_worker, (void *)(intptr_t)1) == thrd_success;
    int have2 = thrd_create(&t2, fp_worker, (void *)(intptr_t)2) == thrd_success;
    EXPECT(have1 && have2);
    int r1 = 0, r2 = 0, j1 = thrd_error, j2 = thrd_error;
    if (have1) {
        j1 = thrd_join(t1, &r1);
    }
    if (have2) {
        j2 = thrd_join(t2, &r2);
    }
    EXPECT(j1 == thrd_success && r1 == 101);
    EXPECT(j2 == thrd_success && r2 == 102);
    EXPECT(bits_of(results[0].final_value) == bits_of(1.0 + WORKER_ROUNDS * 0.25));
    EXPECT(bits_of(results[1].final_value) == bits_of(2.0 + WORKER_ROUNDS * 0.5));
    EXPECT(results[0].probe_failures == 0);
    EXPECT(results[0].probe_status_failures == 0);
    EXPECT(results[0].rounds_done == PROBE_ROUNDS);
    EXPECT(results[1].probe_failures == 0);
    EXPECT(results[1].probe_status_failures == 0);
    EXPECT(results[1].rounds_done == PROBE_ROUNDS);

    /* F3c: raw SYS_SYNC_WAIT transitions, each with its required status.
     * Timing uses >= / far-below predicates only (TCG time dilates). */
    const arena_startup_t *s = arena_startup();
    if (s->present && s->sync_domain != ARENA_ARST_NONE && arena_sync_init() == 0) {
        uint64_t slot = s->caps[s->sync_domain].slot;
        int64_t token = arena_syscall6(ARENA_SYS_SYNC_KEY_CREATE, slot, 0, 0, 0, 0, 0);
        EXPECT(token > 0);
        if (token > 0) {
            /* Parked timeout: nobody wakes the key, so the wait must end
             * by deadline (-8), not before it, and must not advance the
             * sequence. Two rounds. */
            for (int i = 0; i < 2; i++) {
                int64_t seq = arena_syscall6(ARENA_SYS_SYNC_SEQUENCE, slot,
                                             (uint64_t)token, 0, 0, 0, 0);
                EXPECT(seq > 0);
                struct probe_result p = probe_once(
                    ARENA_SYS_SYNC_WAIT, slot, (uint64_t)token, (uint64_t)seq, 1500, 9);
                EXPECT(p.status == ARENA_STATUS_TIMEOUT);
                EXPECT(p.elapsed_us >= 1500);
                EXPECT(p.image == 0);
                int64_t after = arena_syscall6(ARENA_SYS_SYNC_SEQUENCE, slot,
                                               (uint64_t)token, 0, 0, 0, 0);
                EXPECT(after == seq);
            }

            /* Parked wake: the waker observes this thread parked, then
             * wakes exactly one waiter; the wait returns 0. */
            wake_job.slot = slot;
            wake_job.token = (uint64_t)token;
            wake_job.wake_status = -1000;
            wake_job.saw_parked = 0;
            int64_t wake_seq = arena_syscall6(ARENA_SYS_SYNC_SEQUENCE, slot,
                                              (uint64_t)token, 0, 0, 0, 0);
            EXPECT(wake_seq > 0);
            thrd_t waker;
            int have_waker = thrd_create(&waker, wake_worker, NULL) == thrd_success;
            EXPECT(have_waker);
            struct probe_result woken = probe_once(
                ARENA_SYS_SYNC_WAIT, slot, (uint64_t)token, (uint64_t)wake_seq,
                500000, 9);
            EXPECT(woken.status == 0);
            EXPECT(woken.image == 0);
            int waker_result = 0;
            if (have_waker) {
                EXPECT(thrd_join(waker, &waker_result) == thrd_success &&
                       waker_result == 130);
            }
            EXPECT(wake_job.saw_parked == 1);
            EXPECT(wake_job.wake_status == 1); /* exactly one waiter woken */

            /* Stale sequence: wake_seq is now old, so the same wait must
             * return 0 immediately instead of parking. */
            int64_t current = arena_syscall6(ARENA_SYS_SYNC_SEQUENCE, slot,
                                             (uint64_t)token, 0, 0, 0, 0);
            EXPECT(current > wake_seq);
            struct probe_result stale = probe_once(
                ARENA_SYS_SYNC_WAIT, slot, (uint64_t)token, (uint64_t)wake_seq,
                50000, 9);
            EXPECT(stale.status == 0);
            EXPECT(stale.elapsed_us < 50000);
            EXPECT(stale.image == 0);

            /* Immediate-refusal negative control: a bad token is refused
             * at once (STATUS_BAD_ARG). This is deliberately NOT a
             * blocking proof; it pins that immediate errors are visible
             * in the status and cannot masquerade as a parked wait. */
            struct probe_result refused = probe_once(
                ARENA_SYS_SYNC_WAIT, slot, UINT64_MAX, (uint64_t)current, 50000, 9);
            EXPECT(refused.status == ARENA_STATUS_BAD_ARG);
            EXPECT(refused.elapsed_us < 50000);
            EXPECT(refused.image == 0);

            EXPECT(arena_syscall6(ARENA_SYS_SYNC_KEY_DESTROY, slot,
                                  (uint64_t)token, 0, 0, 0, 0) == 0);
        }
    } else {
        skips++;
        printf(APP_NAME " F3c SKIP (no sync domain granted)\n");
    }

    /* F4: preemption exposure (see header for the exact claim). */
    thrd_t s1, s2;
    int hs1 = thrd_create(&s1, spin_worker, (void *)(intptr_t)1) == thrd_success;
    int hs2 = thrd_create(&s2, spin_worker, (void *)(intptr_t)2) == thrd_success;
    EXPECT(hs1 && hs2);
    spin_fp(30000);
    int q1 = 0, q2 = 0;
    if (hs1) {
        EXPECT(thrd_join(s1, &q1) == thrd_success && q1 == 121);
    }
    if (hs2) {
        EXPECT(thrd_join(s2, &q2) == thrd_success && q2 == 122);
    }
    expect_probe(probe_once(0, 0, 0, 0, 0, 9), 0); /* main state still exact */

    /* F5: formatting contract unchanged. */
    char buf[64];
    int n = snprintf(buf, sizeof buf, "%d|%x", -7, 48879u);
    EXPECT(n == (int)strlen(buf) && strcmp(buf, "-7|beef") == 0);
    char fbuf[32] = "untouched";
    errno = 0;
    EXPECT(snprintf(fbuf, sizeof fbuf, "%f", 1.5) == -1 && errno == ENOTSUP);

    printf(APP_NAME " VERDICT %s checks=%d failed=%d skips=%d\n",
           failed_checks == 0 ? "PASS" : "FAIL", checks, failed_checks, skips);
    return failed_checks == 0 ? EXIT_PASS : EXIT_FAIL;
}
