/*
 * C2.5 App A: console program on the reusable ArenaOS C runtime (EXPERIMENTAL).
 *
 * Uses ONLY the standard C headers provided by the C2 SDK (include/libc) plus
 * the documented arena/rt.h runtime interface. No floating point (the no-FP
 * profile), no file access, no graphics. Groups:
 *   G1 stdio      formatting, return values, bounded snprintf, stderr, puts/putchar
 *   G2 stdin      granted standard input, non-blocking classification, write refusal
 *   G3 allocator  malloc/calloc/realloc/free/aligned_alloc, overflow, accounting
 *   G4 time       CLOCK_MONOTONIC ordering and nanosleep; REALTIME refused (ENOSYS)
 *   G5 threads    thrd_create/join, mtx, cnd_timedwait timeout, call_once, TLS
 *   G6 threads ext  trylock busy, cnd_wait/broadcast wakeup, detach, thrd_sleep,
 *                 documented refusals (recursive mutex, timed lock, tss keys)
 *   G7 stdio ext  fputc/putc/fwrite, error and EOF flags, setvbuf range
 *   G8 init       a constructor (.init_array) ran before main
 *   G9 exit       atexit handler runs after main returns
 * Exit status: 61 when every group passes, 62 otherwise, 58 if stdio cannot attach.
 */
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <threads.h>
#include <time.h>

#include "arena/abi.h"
#include "arena/rt.h"

#define APP_NAME "[c2-console]"
#define EXIT_PASS 61
#define EXIT_FAIL 62
#define EXIT_NO_STREAMS 58
#define GROUPS 9

static int groups_passed;
static int checks;
static int failed_checks;
static volatile int atexit_ran;
static int atexit_registered;

static void group_result(const char *name, int ok, int count) {
    if (ok) {
        groups_passed++;
    }
    printf(APP_NAME " %s %s checks=%d\n", name, ok ? "PASS" : "FAIL", count);
}

#define EXPECT(cond, ...)                                                       \
    do {                                                                        \
        checks++;                                                               \
        if (!(cond)) {                                                          \
            failed_checks++;                                                    \
            printf(APP_NAME " check failed line %d: " #cond "\n", __LINE__);    \
        }                                                                       \
    } while (0)

/* ------------------------------------------------------------- G1 stdio ---- */
static void g1_stdio(void) {
    int before = failed_checks, n0 = checks;
    char b[160];
    int n = snprintf(b, sizeof b, "%d|%5.2d|%-4s|%x|%llu|%c", -42, 7, "ab", 255u,
                     1234567890123ULL, 'Z');
    EXPECT(n == (int)strlen(b) && strcmp(b, "-42|   07|ab  |ff|1234567890123|Z") == 0,
           "snprintf fields: %s", b);
    char small[4];
    n = snprintf(small, sizeof small, "%d", 123456);
    EXPECT(n == 6 && strcmp(small, "123") == 0, "snprintf truncation return and text");
    /* Floating point is outside the C2 profile: refused with ENOTSUP, no output.
     * The argument is never consumed, so an integer placeholder is passed. */
    errno = 0;
    n = snprintf(b, sizeof b, "%f", 0);
    EXPECT(n == -1 && errno == ENOTSUP, "floating-point conversion refused (ENOTSUP)");
    errno = 0;
    n = snprintf(b, sizeof b, "%y", 0);
    EXPECT(n == 2 && strcmp(b, "%y") == 0, "unknown conversion echoed (C1 contract)");
    n = fprintf(stderr, APP_NAME " stderr channel active\n");
    EXPECT(n > 0, "stderr write returns a positive count");
    EXPECT(puts("[c2-console] puts line") >= 0, "puts");
    EXPECT(putchar('.') == '.', "putchar returns the character");
    EXPECT(putchar('\n') == '\n', "putchar newline");
    EXPECT(fputs("[c2-console] fputs line\n", stdout) >= 0, "fputs");
    group_result("G1 stdio", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------------ G2 stdin ----- */
static void g2_stdin(void) {
    int before = failed_checks, n0 = checks;
    unsigned char buf[16];
    long r = arena_stream_read_some(0, buf, sizeof buf);
    /* Non-blocking read of the granted stdin: a block, end of stream, or data
     * are all valid; the count must stay in bounds. */
    EXPECT(r == ARENA_E_WOULD_BLOCK || r == 0 || (r > 0 && r <= (long)sizeof buf),
           "stdin read is non-blocking and bounded (r=%ld)", r);
    EXPECT(arena_stream_write(0, "x", 1) == ARENA_E_INVALID, "stdin is not writable");
    printf(APP_NAME " stdin class=%s\n",
           r == ARENA_E_WOULD_BLOCK ? "would-block" : (r == 0 ? "eof" : "data"));
    group_result("G2 stdin", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------------ G3 heap ------ */
static void g3_allocator(void) {
    int before = failed_checks, n0 = checks;
    struct arena_heap_stats s0, s1, s2;
    arena_heap_stats(&s0);

    unsigned char *z = calloc(64, 8);
    EXPECT(z != NULL, "calloc");
    int zeroed = 1;
    for (size_t i = 0; i < 512; i++) {
        if (z[i] != 0) {
            zeroed = 0;
        }
    }
    EXPECT(zeroed, "calloc zeroes its block");

    unsigned char *p = malloc(100);
    EXPECT(p != NULL, "malloc");
    if (p != NULL) {
        memset(p, 0x5a, 100);
        unsigned char *q = realloc(p, 4000);
        EXPECT(q != NULL, "realloc grow");
        if (q != NULL) {
            EXPECT(q[0] == 0x5a && q[99] == 0x5a, "realloc preserves contents");
            p = q;
        }
        free(p);
    }

    void *al = aligned_alloc(4096, 10000);
    EXPECT(al != NULL && ((uintptr_t)al % 4096) == 0, "aligned_alloc(4096) alignment");
    free(al);
    errno = 0;
    EXPECT(aligned_alloc(3, 8) == NULL && errno == EINVAL, "non-power-of-two alignment refused");
    errno = 0;
    EXPECT(calloc((size_t)-1, 2) == NULL && errno == ENOMEM, "calloc overflow refused");
    free(z);
    free(NULL);

    arena_heap_stats(&s1);
    arena_heap_stats(&s2);
    EXPECT(s2.live_blocks == s0.live_blocks, "live blocks return to baseline (%llu vs %llu)",
           (unsigned long long)s2.live_blocks, (unsigned long long)s0.live_blocks);
    group_result("G3 allocator", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------------ G4 time ------ */
static void g4_time(void) {
    int before = failed_checks, n0 = checks;
    struct timespec a, b, req = {0, 2000000};
    EXPECT(clock_gettime(CLOCK_MONOTONIC, &a) == 0, "monotonic read");
    EXPECT(nanosleep(&req, NULL) == 0, "nanosleep 2 ms");
    EXPECT(clock_gettime(CLOCK_MONOTONIC, &b) == 0, "monotonic read after sleep");
    long long da = (long long)a.tv_sec * 1000000000LL + a.tv_nsec;
    long long db = (long long)b.tv_sec * 1000000000LL + b.tv_nsec;
    EXPECT(db - da >= 2000000LL, "monotonic clock advanced by at least 2 ms (%lld ns)", db - da);
    errno = 0;
    EXPECT(clock_gettime(CLOCK_REALTIME, &a) == -1 && errno == ENOSYS,
           "CLOCK_REALTIME refused (no wall-clock authority)");
    group_result("G4 time", failed_checks == before, checks - n0);
}

/* ---------------------------------------------------------- G5 threads ----- */
static mtx_t counter_lock;
static long shared_counter;
static cnd_t never_signalled;
static once_flag once = ONCE_FLAG_INIT;
static int once_calls;
static thread_local int tls_value;

static void bump_once(void) { once_calls++; }

static int worker(void *arg) {
    int id = (int)(intptr_t)arg;
    tls_value = id * 100;
    for (int i = 0; i < 500; i++) {
        if (mtx_lock(&counter_lock) != thrd_success) {
            return -1;
        }
        shared_counter++;
        mtx_unlock(&counter_lock);
    }
    call_once(&once, bump_once);
    thrd_yield();
    /* TLS isolation: this thread still sees its own value. */
    return tls_value == id * 100 ? id * 10 + 1 : -2;
}

static void g5_threads(void) {
    int before = failed_checks, n0 = checks;
    thrd_t t[3];
    int created = 0;
    EXPECT(mtx_init(&counter_lock, mtx_plain) == thrd_success, "mutex init");
    EXPECT(cnd_init(&never_signalled) == thrd_success, "condvar init");
    for (int i = 0; i < 3; i++) {
        if (thrd_create(&t[i], worker, (void *)(intptr_t)(i + 1)) == thrd_success) {
            created++;
        }
    }
    EXPECT(created == 3, "three threads created");
    int ok_status = 0;
    for (int i = 0; i < created; i++) {
        int res = -99;
        if (thrd_join(t[i], &res) == thrd_success && res == (i + 1) * 10 + 1) {
            ok_status++;
        }
    }
    EXPECT(ok_status == 3, "each thread returned its own status (%d of 3)", ok_status);
    EXPECT(shared_counter == 1500, "mutex-protected counter is exact (%ld)", shared_counter);
    call_once(&once, bump_once);
    EXPECT(once_calls == 1, "call_once ran exactly once (%d)", once_calls);
    tls_value = 7;
    EXPECT(tls_value == 7, "main thread TLS is independent");

    /* Timed wait with nothing signalling: must time out, not hang. */
    struct timespec deadline;
    EXPECT(clock_gettime(CLOCK_MONOTONIC, &deadline) == 0, "deadline base");
    deadline.tv_nsec += 5000000L;
    if (deadline.tv_nsec >= 1000000000L) {
        deadline.tv_sec += 1;
        deadline.tv_nsec -= 1000000000L;
    }
    mtx_lock(&counter_lock);
    int wr = cnd_timedwait(&never_signalled, &counter_lock, &deadline);
    mtx_unlock(&counter_lock);
    EXPECT(wr == thrd_timedout, "cnd_timedwait times out (%d)", wr);
    group_result("G5 threads", failed_checks == before, checks - n0);
}


/* ------------------------------------------------- G6 threads, extended ----- */
static mtx_t gate_lock;
static cnd_t gate_cv;
static int gate_open;
static volatile int detached_done;

static int gate_waiter(void *arg) {
    (void)arg;
    if (mtx_lock(&gate_lock) != thrd_success) {
        return -1;
    }
    while (!gate_open) {
        if (cnd_wait(&gate_cv, &gate_lock) != thrd_success) {
            mtx_unlock(&gate_lock);
            return -2;
        }
    }
    mtx_unlock(&gate_lock);
    return 7;
}

static int detached_worker(void *arg) {
    (void)arg;
    detached_done = 1;
    return 0;
}

static void g6_threads_ext(void) {
    int before = failed_checks, n0 = checks;
    mtx_t self_lock, timed_lock;
    EXPECT(thrd_current() != NULL && thrd_equal(thrd_current(), thrd_current()) != 0,
           "thrd_current and thrd_equal on the main thread");

    /* Trylock: free -> success; held by this thread (non-recursive) -> busy. */
    EXPECT(mtx_init(&self_lock, mtx_plain) == thrd_success, "trylock mutex init");
    EXPECT(mtx_trylock(&self_lock) == thrd_success, "trylock on a free mutex succeeds");
    EXPECT(mtx_trylock(&self_lock) == thrd_busy, "trylock on a self-held mutex is busy");
    EXPECT(mtx_unlock(&self_lock) == thrd_success, "unlock after trylock");
    mtx_destroy(&self_lock);

    /* Refusals must be explicit errors, never success. */
    EXPECT(mtx_init(&timed_lock, mtx_plain | mtx_recursive) == thrd_error,
           "recursive mutex refused at init");
    EXPECT(mtx_init(&timed_lock, mtx_plain) == thrd_success, "timed-lock mutex init");
    struct timespec soon;
    EXPECT(clock_gettime(CLOCK_MONOTONIC, &soon) == 0, "timed-lock deadline base");
    soon.tv_nsec += 1000000L;
    if (soon.tv_nsec >= 1000000000L) {
        soon.tv_sec += 1;
        soon.tv_nsec -= 1000000000L;
    }
    EXPECT(mtx_timedlock(&timed_lock, &soon) == thrd_error, "mtx_timedlock is refused");
    mtx_destroy(&timed_lock);
    tss_t key;
    EXPECT(tss_create(&key, NULL) == thrd_error, "tss_create is refused");
    EXPECT(tss_get(key) == NULL, "tss_get reports no value (refused key)");
    EXPECT(tss_set(key, (void *)1) == thrd_error, "tss_set is refused");

    /* Condition variable wakeup: waiter parks until the gate opens. The predicate
     * loop makes the result independent of which side runs first. */
    EXPECT(mtx_init(&gate_lock, mtx_plain) == thrd_success, "gate mutex init");
    EXPECT(cnd_init(&gate_cv) == thrd_success, "gate condvar init");
    thrd_t waiter;
    EXPECT(thrd_create(&waiter, gate_waiter, NULL) == thrd_success, "gate waiter created");
    EXPECT(mtx_lock(&gate_lock) == thrd_success, "gate lock");
    gate_open = 1;
    EXPECT(cnd_broadcast(&gate_cv) == thrd_success, "cnd_broadcast returns success");
    EXPECT(mtx_unlock(&gate_lock) == thrd_success, "gate unlock");
    int res = -99;
    EXPECT(thrd_join(waiter, &res) == thrd_success && res == 7,
           "cnd_wait woke after broadcast (status %d)", res);
    EXPECT(cnd_signal(&gate_cv) == thrd_success, "cnd_signal with no waiters succeeds");
    cnd_destroy(&gate_cv);
    mtx_destroy(&gate_lock);

    /* Detach: the thread runs to completion without a join. Bounded wait. */
    thrd_t quiet;
    EXPECT(thrd_create(&quiet, detached_worker, NULL) == thrd_success, "detach candidate created");
    EXPECT(thrd_detach(quiet) == thrd_success, "thrd_detach succeeds");
    int spins = 0;
    while (!detached_done && spins < 2000) {
        (void)thrd_sleep(&(struct timespec){.tv_sec = 0, .tv_nsec = 1000000L}, NULL);
        spins++;
    }
    EXPECT(detached_done == 1, "detached thread ran (%d spins)", spins);

    /* thrd_sleep delegates to nanosleep: 1 ms returns success. */
    EXPECT(thrd_sleep(&(struct timespec){.tv_sec = 0, .tv_nsec = 1000000L}, NULL) == thrd_success,
           "thrd_sleep 1 ms");
    group_result("G6 threads ext", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------ G7 stdio, extended -- */
static void g7_stdio_ext(void) {
    int before = failed_checks, n0 = checks;
    EXPECT(fputc('A', stdout) == 'A', "fputc returns the character");
    EXPECT(putc('B', stdout) == 'B', "putc returns the character");
    EXPECT(fwrite("CD", 1, 2, stdout) == 2, "fwrite returns the item count");
    printf("\n");
    EXPECT(ferror(stdout) == 0 && feof(stdout) == 0, "stdout starts with no error or EOF");
    clearerr(stdout);
    EXPECT(ferror(stdout) == 0, "clearerr leaves no error");
    EXPECT(setvbuf(stdout, NULL, _IOLBF, 0) == 0, "setvbuf accepts a valid mode");
    errno = 0;
    EXPECT(setvbuf(stdout, NULL, 9, 0) == -1 && errno == EINVAL, "setvbuf refuses a bad mode");
    perror("[c2-console] perror probe");
    group_result("G7 stdio ext", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------ G8 init constructor -- */
static int ctor_ran;
__attribute__((constructor)) static void early_ctor(void) { ctor_ran = 1; }

static void g8_init(void) {
    int before = failed_checks, n0 = checks;
    EXPECT(ctor_ran == 1, "constructor in .init_array ran before main");
    group_result("G8 init", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------------ G9 exit ------ */
static void on_exit_handler(void) {
    atexit_ran = 1;
    printf(APP_NAME " atexit handler ran\n");
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    /* Without a stream grant nothing can be reported; the defined status is the signal. */
    if (arena_stdio_init() != 0) {
        return EXIT_NO_STREAMS;
    }
    atexit_registered = atexit(on_exit_handler) == 0;
    printf(APP_NAME " C2 console application entered (ARST v2 gate)\n");
    g1_stdio();
    g2_stdin();
    g3_allocator();
    g4_time();
    g5_threads();
    g6_threads_ext();
    g7_stdio_ext();
    g8_init();
    /* G9 exit: registration succeeded. The handler runs after main returns; the
     * harness requires its line AFTER the RESULT line (ordering = exit path). */
    group_result("G9 exit", atexit_registered, 1);
    int pass = groups_passed == GROUPS && failed_checks == 0;
    printf(APP_NAME " RESULT %s groups=%d/%d checks=%d failed=%d\n", pass ? "PASS" : "FAIL",
           groups_passed, GROUPS, checks, failed_checks);
    /* The streams are flushed and closed by the runtime's exit path, after the
     * atexit handlers (C11 order). */
    return pass ? EXIT_PASS : EXIT_FAIL;
}
