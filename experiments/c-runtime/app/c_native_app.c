/*
 * C1.2 / C1.3 / C1.4 native C application: an ordinary signed C program that
 * the desktop launches through the normal headless lifecycle (APB1 install,
 * registry launch, ARST v2 startup record with FLAG_HEADLESS|STREAMS|SYNC).
 *
 * It is NOT a boot probe. It is not spawned from the kernel, not patched in,
 * and has no privileged route. Every capability it uses comes from the
 * startup record the desktop granted.
 *
 * Output: granted stdout/stderr streams. Exit: 57 when every group passes,
 * 1 otherwise. The harness requires the exact RESULT line AND exit status 57.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "arena/rt.h"
#include "arena/string.h"

#define APP_ID "org.arenaos.cnative"
#define EXIT_PASS 57
#define EXIT_FAIL 1
#define GROUPS 9

static int groups_passed;
static int checks;
static int failed_checks;

static void group_result(const char *name, int ok, int count) {
    if (ok) {
        groups_passed++;
    }
    arena_printf("[c-native] %s %s checks=%d\n", name, ok ? "PASS" : "FAIL", count);
}

#define EXPECT(cond, ...)                                                     \
    do {                                                                       \
        checks++;                                                              \
        if (!(cond)) {                                                         \
            failed_checks++;                                                   \
            arena_printf("[c-native] check failed line %d: " #cond "\n", __LINE__); \
        }                                                                      \
    } while (0)

/* ----------------------------------------------------------- T1 startup -- */
static void t1_startup(void) {
    int before = failed_checks, n0 = checks;
    const arena_startup_t *s = arena_startup();
    EXPECT(s->present == 1, "record");
    EXPECT(s->flags == (ARENA_ARST_FLAG_HEADLESS | ARENA_ARST_FLAG_STANDARD_STREAMS |
                        ARENA_ARST_FLAG_NATIVE_SYNC), "flags");
    EXPECT(s->argc == 1 && s->argv != NULL && arena_strcmp(s->argv[0], APP_ID) == 0, "argv");
    EXPECT(s->stream_set != ARENA_ARST_NONE && s->stream_wake != ARENA_ARST_NONE, "stream roles");
    EXPECT(s->sync_domain != ARENA_ARST_NONE && s->notification != ARENA_ARST_NONE, "sync roles");
    EXPECT(arena_stdio_init() == 0, "stdio attach");
    EXPECT(arena_sync_init() == 0, "sync bind");
    EXPECT(arena_stdio_init() == 0, "stdio attach is idempotent");
    EXPECT(arena_tls_ready() == 1, "tls");
    uintptr_t lo = 0, hi = 0;
    arena_stack_bounds(&lo, &hi);
    EXPECT(hi - lo == 256 * 1024, "stack size");
    group_result("T1 startup-abi-v2", failed_checks == before, checks - n0);
}

/* -------------------------------------------------- T2 granted stdout/err -- */
static void t2_streams(void) {
    int before = failed_checks, n0 = checks;
    static const char line[] = "[c-native] stdout line through the granted ring\n";
    long n = arena_stream_write(1, line, sizeof line - 1);
    EXPECT(n == (long)(sizeof line - 1), "stdout exact write");
    /* 2 KiB total exceeds the 768-byte ring, so this crosses backpressure
     * (WOULD_BLOCK, SYS_WAIT on the granted notification, desktop drain). */
    char chunk[64];
    arena_memset(chunk, 'k', sizeof chunk);
    size_t total = 0;
    for (unsigned i = 0; i < 32; i++) {
        long w = arena_stream_write(1, chunk, sizeof chunk);
        if (w == (long)sizeof chunk) {
            total += (size_t)w;
        }
    }
    EXPECT(total == 2048, "backpressure transfer");
    arena_printf("[c-native] backpressure: %lu bytes accepted over the bounded stdout ring\n",
                 (unsigned long)total);
    static const char err[] = "[c-native] stderr channel reached the broker\n";
    EXPECT(arena_stream_write(2, err, sizeof err - 1) == (long)(sizeof err - 1), "stderr write");
    EXPECT(arena_stream_write(0, err, 1) == ARENA_E_INVALID, "stdin is not writable");
    EXPECT(arena_stream_write(7, err, 1) == ARENA_E_INVALID, "out-of-range fd refused");
    group_result("T2 granted-streams", failed_checks == before, checks - n0);
}

/* --------------------------------------------------- T3 stdin (non-block) -- */
static void t3_stdin(void) {
    int before = failed_checks, n0 = checks;
    char buf[16];
    long r = arena_stream_read_some(0, buf, sizeof buf);
    EXPECT(r == ARENA_E_WOULD_BLOCK || r == 0 || r > 0, "stdin read class");
    EXPECT(r <= (long)sizeof buf, "stdin bounded");
    arena_printf("[c-native] stdin nonblocking read class=%s\n",
                 r == ARENA_E_WOULD_BLOCK ? "would-block" : (r == 0 ? "eof" : "bytes"));
    group_result("T3 stdin-bounded-read", failed_checks == before, checks - n0);
}

/* ---------------------------------------- T4 ring protocol and concurrency -- */
static uint8_t loop_page[ARENA_STREAM_PAGE_BYTES] __attribute__((aligned(4096)));
static uint8_t loop_page2[ARENA_STREAM_PAGE_BYTES] __attribute__((aligned(4096)));
#define LOOP_TOTAL (64u * 1024u)

static int loop_producer(void *arg) {
    uint8_t *page = (uint8_t *)arg;
    uint8_t block[97];
    uint32_t sent = 0;
    while (sent < LOOP_TOTAL) {
        size_t want = LOOP_TOTAL - sent < sizeof block ? LOOP_TOTAL - sent : sizeof block;
        for (size_t i = 0; i < want; i++) {
            block[i] = (uint8_t)((sent + i) * 31u + 7u);
        }
        long n = arena_ring_write_some(page, 1, block, want);
        if (n > 0) {
            sent += (uint32_t)n;
        } else if (n == ARENA_E_WOULD_BLOCK) {
            arena_yield();
        } else {
            return 1;
        }
    }
    arena_ring_close_writer(page, 1);
    return 0;
}

static void t4_rings(void) {
    int before = failed_checks, n0 = checks;
    EXPECT(arena_ring_page_init(loop_page) == 0, "page init");
    EXPECT(arena_ring_page_check(loop_page) == 0, "page check");
    uint8_t src[1400];
    for (unsigned i = 0; i < sizeof src; i++) {
        src[i] = (uint8_t)(i * 13u + 1u);
    }
    /* Partial transfer: 1000 bytes into a 768-byte ring accepts 768. */
    EXPECT(arena_ring_write_some(loop_page, 1, src, sizeof src) == 768, "partial transfer");
    EXPECT(arena_ring_write_some(loop_page, 1, src, 1) == ARENA_E_WOULD_BLOCK, "full ring");
    uint8_t dst[1000];
    EXPECT(arena_ring_read_some(loop_page, 1, dst, 100) == 100, "bounded read");
    /* Freed space is 100 bytes: the next write is short-counted to 100. */
    EXPECT(arena_ring_write_some(loop_page, 1, src + 768, 300) == 100, "short write after drain");
    /* The ring now holds 668 + 100 = 768 bytes: one read drains all of them. */
    EXPECT(arena_ring_read_some(loop_page, 1, dst + 100, 768) == 768, "drain remaining");
    int ordered = 1;
    for (unsigned i = 0; i < 768; i++) {
        if (dst[i] != src[i]) {
            ordered = 0;
        }
    }
    for (unsigned i = 768; i < 868; i++) {
        if (dst[i] != src[i]) {
            ordered = 0;
        }
    }
    EXPECT(ordered, "byte-exact FIFO order");
    /* EOF is only after the writer closes and the ring drains. */
    EXPECT(arena_ring_read_some(loop_page, 1, dst, 4) == ARENA_E_WOULD_BLOCK, "open empty ring");
    arena_ring_close_writer(loop_page, 1);
    EXPECT(arena_ring_read_some(loop_page, 1, dst, 4) == 0, "eof after close");
    EXPECT(arena_ring_write_some(loop_page, 1, src, 1) == ARENA_E_CLOSED, "write after own close");
    /* Peer closure: the reader closes, writes get BROKEN_PIPE. */
    EXPECT(arena_ring_page_init(loop_page2) == 0, "second page");
    arena_ring_close_reader(loop_page2, 2);
    EXPECT(arena_ring_write_some(loop_page2, 2, src, 1) == ARENA_E_BROKEN_PIPE, "broken pipe");

    /* Real concurrency: a producer thread and this thread share loop_page2. */
    arena_ring_page_init(loop_page2);
    arena_thread_t producer;
    EXPECT(arena_thread_create(&producer, loop_producer, loop_page2) == 0, "producer create");
    uint32_t got = 0;
    int bytes_ok = 1;
    while (1) {
        uint8_t chunk[61];
        long n = arena_ring_read_some(loop_page2, 1, chunk, sizeof chunk);
        if (n > 0) {
            for (long i = 0; i < n; i++) {
                if (chunk[i] != (uint8_t)((got + (uint32_t)i) * 31u + 7u)) {
                    bytes_ok = 0;
                }
            }
            got += (uint32_t)n;
        } else if (n == 0) {
            break;
        } else if (n == ARENA_E_WOULD_BLOCK) {
            arena_yield();
        } else {
            bytes_ok = 0;
            break;
        }
    }
    int pstatus = -1;
    EXPECT(arena_thread_join(&producer, &pstatus) == 0 && pstatus == 0, "producer joined ok");
    EXPECT(got == LOOP_TOTAL, "all bytes delivered before EOF");
    EXPECT(bytes_ok, "concurrent bytes in order");
    group_result("T4 ring-protocol", failed_checks == before, checks - n0);
}

/* ------------------------------------------------ T5 allocator hardening -- */
static void t5_allocator(void) {
    int before = failed_checks, n0 = checks;
    struct arena_heap_stats base, st;
    arena_heap_stats(&base);
    uint8_t *p = arena_malloc(100);
    EXPECT(p != NULL && ((uintptr_t)p & 15u) == 0, "aligned alloc");
    for (unsigned i = 0; p && i < 100; i++) {
        p[i] = (uint8_t)(i ^ 0x5Au);
    }
    uint8_t *q = arena_realloc(p, 5000);
    EXPECT(q != NULL, "growing realloc");
    int kept = 1;
    for (unsigned i = 0; q && i < 100; i++) {
        if (q[i] != (uint8_t)(i ^ 0x5Au)) {
            kept = 0;
        }
    }
    EXPECT(kept, "realloc preserves bytes");
    /* Inside the block's capacity: same pointer, no copy. */
    uint8_t *r = arena_realloc(q, 10);
    EXPECT(r == q, "in-capacity realloc keeps the block");
    arena_heap_stats(&st);
    uint64_t bad_before = st.bad_frees;
    /* Invalid pointers are rejected and counted, never trusted. */
    arena_free(q + 16);
    int stack_local = 0;
    arena_free(&stack_local);
    arena_free(NULL);
    arena_heap_stats(&st);
    EXPECT(st.bad_frees == bad_before + 2, "interior and foreign pointers rejected");
    arena_free(q);
    arena_free(q);
    arena_heap_stats(&st);
    EXPECT(st.bad_frees == bad_before + 3, "double free rejected");
    EXPECT(st.live_blocks == base.live_blocks, "live blocks return to baseline");
    EXPECT(arena_calloc((size_t)-1, 2) == NULL, "calloc overflow refused");
    EXPECT(arena_realloc(NULL, 0) != NULL, "malloc(0) returns a block");
    uint8_t *big = arena_malloc(1024 * 1024);
    EXPECT(big != NULL, "1 MiB allocation");
    if (big) {
        big[0] = 1;
        big[1024 * 1024 - 1] = 2;
        EXPECT(big[0] == 1 && big[1024 * 1024 - 1] == 2, "large block usable");
        arena_free(big);
    }
    EXPECT(arena_malloc((size_t)1 << 40) == NULL, "oversize allocation refused");
    arena_heap_stats(&st);
    EXPECT(st.refused >= base.refused + 2, "refusals counted");
    group_result("T5 allocator-hardening", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------- T6 monotonic time -- */
static void t6_time(void) {
    int before = failed_checks, n0 = checks;
    uint64_t t0 = arena_clock_us();
    uint64_t prev = t0;
    int monotonic = 1;
    for (unsigned i = 0; i < 2000; i++) {
        uint64_t now = arena_clock_us();
        if (now < prev) {
            monotonic = 0;
        }
        prev = now;
    }
    EXPECT(monotonic, "clock never goes backwards");
    arena_busy_sleep_us(20000);
    uint64_t t1 = arena_clock_us();
    EXPECT(t1 - t0 >= 20000, "sleep covers at least 20 ms");
    EXPECT(t1 - t0 < 2000000, "sleep bounded");
    arena_printf("[c-native] monotonic clock: slept %lu us\n", (unsigned long)(t1 - t0));
    group_result("T6 monotonic-time", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------------- T7 logic -- */
static void t7_logic(void) {
    int before = failed_checks, n0 = checks;
    static uint8_t composite[10001];
    unsigned primes = 0;
    for (unsigned i = 2; i <= 10000; i++) {
        if (!composite[i]) {
            primes++;
            for (unsigned j = i * i; j <= 10000; j += i) {
                composite[j] = 1;
            }
        }
    }
    EXPECT(primes == 1229, "pi(10000)");
    uint64_t a = 0, b = 1;
    for (unsigned i = 0; i < 90; i++) {
        uint64_t t = a + b;
        a = b;
        b = t;
    }
    EXPECT(a == 2880067194370816120ULL, "fib(90)");
    EXPECT((int)((-7) / 2) == -3 && (unsigned)(0xFFFFFFFFu) + 1u == 0u, "integer semantics");
    group_result("T7 logic", failed_checks == before, checks - n0);
}

/* -------------------------------------------------------- T8 TLS isolation -- */
static __thread int tls_value = 7;

static int tls_worker(void *arg) {
    int want = (int)(intptr_t)arg;
    tls_value = want;
    arena_busy_sleep_us(2000);
    return tls_value == want ? 0 : 1;
}

static void t8_tls(void) {
    int before = failed_checks, n0 = checks;
    arena_thread_t a, b;
    EXPECT(arena_thread_create(&a, tls_worker, (void *)(intptr_t)11) == 0, "tls thread a");
    EXPECT(arena_thread_create(&b, tls_worker, (void *)(intptr_t)22) == 0, "tls thread b");
    int sa = -1, sb = -1;
    EXPECT(arena_thread_join(&a, &sa) == 0 && sa == 0, "thread a kept its own TLS");
    EXPECT(arena_thread_join(&b, &sb) == 0 && sb == 0, "thread b kept its own TLS");
    EXPECT(tls_value == 7, "main thread TLS unchanged");
    group_result("T8 per-thread-tls", failed_checks == before, checks - n0);
}

/* --------------------------------------- T9 threads, mutex, condvar, once -- */
static arena_mutex_t counter_lock;
static uint64_t shared_counter;
static arena_once_t once_gate;
static int once_runs;

static void once_init(void) {
    once_runs++;
}

static int counter_worker(void *arg) {
    unsigned id = (unsigned)(uintptr_t)arg;
    for (unsigned i = 0; i < 2500; i++) {
        if (arena_mutex_lock(&counter_lock) != 0) {
            return 1;
        }
        shared_counter++;
        (void)arena_mutex_unlock(&counter_lock);
    }
    (void)arena_once_call(&once_gate, once_init);
    return (int)(id * 10u + 1u);
}

static arena_mutex_t gate_lock;
static arena_condvar_t gate_cv;
static int gate_open;

static int gated_worker(void *arg) {
    (void)arg;
    (void)arena_mutex_lock(&gate_lock);
    while (!gate_open) {
        if (arena_condvar_wait(&gate_cv, &gate_lock) != 0) {
            (void)arena_mutex_unlock(&gate_lock);
            return 2;
        }
    }
    (void)arena_mutex_unlock(&gate_lock);
    return 0;
}

static int waiter_worker(void *arg) {
    (void)arg;
    (void)arena_mutex_lock(&gate_lock);
    int timed_out = 0;
    /* Timed wait with no notify must time out, then the real notify wakes it. */
    (void)arena_condvar_wait_timeout(&gate_cv, &gate_lock, 5000, &timed_out);
    int saw_timeout = timed_out;
    while (!gate_open) {
        if (arena_condvar_wait(&gate_cv, &gate_lock) != 0) {
            (void)arena_mutex_unlock(&gate_lock);
            return 3;
        }
    }
    (void)arena_mutex_unlock(&gate_lock);
    return saw_timeout ? 42 : 4;
}

static int quiet_worker(void *arg) {
    (void)arg;
    return 5;
}

static void t9_sync(void) {
    int before = failed_checks, n0 = checks;
    uint64_t keys_base = 0;
    EXPECT(arena_sync_key_count(&keys_base) == 0, "key count readable");
    EXPECT(arena_mutex_init(&counter_lock) == 0, "counter mutex");
    EXPECT(arena_once_init(&once_gate) == 0, "once");
    EXPECT(arena_mutex_init(&gate_lock) == 0 && arena_condvar_init(&gate_cv) == 0, "gate primitives");

    /* Four concurrent workers: join results, mutex-protected counter, once. */
    arena_thread_t workers[4];
    int created = 0;
    for (unsigned i = 0; i < 4; i++) {
        if (arena_thread_create(&workers[i], counter_worker, (void *)(uintptr_t)i) == 0) {
            created++;
        }
    }
    EXPECT(created == 4, "four workers created");
    int joined_ok = 1;
    for (unsigned i = 0; i < 4; i++) {
        int st = -1;
        if (arena_thread_join(&workers[i], &st) != 0 || st != (int)(i * 10u + 1u)) {
            joined_ok = 0;
        }
    }
    EXPECT(joined_ok, "join results match each worker");
    EXPECT(shared_counter == 10000, "mutex-protected counter exact");
    EXPECT(once_runs == 1, "one-time init ran exactly once");

    /* Thread quota: exactly four user threads are gated alive, the fifth is
     * refused by the kernel (STATUS_QUOTA). */
    arena_thread_t held[4];
    int held_ok = 0;
    for (unsigned i = 0; i < 4; i++) {
        if (arena_thread_create(&held[i], gated_worker, NULL) == 0) {
            held_ok++;
        }
    }
    EXPECT(held_ok == 4, "four gated threads");
    EXPECT(arena_thread_count() == 5, "main plus four user threads live");
    arena_thread_t extra;
    EXPECT(arena_thread_create(&extra, quiet_worker, NULL) == ARENA_STATUS_QUOTA,
           "fifth user thread refused with QUOTA");
    /* Condvar wakeup: open the gate and notify every gated waiter. */
    (void)arena_mutex_lock(&gate_lock);
    gate_open = 1;
    (void)arena_mutex_unlock(&gate_lock);
    EXPECT(arena_condvar_notify_all(&gate_cv) >= 0, "notify_all");
    int gated_ok = 1;
    for (unsigned i = 0; i < 4; i++) {
        int st = -1;
        if (arena_thread_join(&held[i], &st) != 0 || st != 0) {
            gated_ok = 0;
        }
    }
    EXPECT(gated_ok, "all gated threads woke and joined");
    EXPECT(arena_thread_count() == 1, "thread count returns to main only");

    /* Timed wait and notify_one wakeup in a separate waiter thread. */
    gate_open = 0;
    arena_thread_t waiter;
    EXPECT(arena_thread_create(&waiter, waiter_worker, NULL) == 0, "waiter create");
    arena_busy_sleep_us(20000);
    (void)arena_mutex_lock(&gate_lock);
    gate_open = 1;
    (void)arena_mutex_unlock(&gate_lock);
    EXPECT(arena_condvar_notify_one(&gate_cv) == 1, "notify_one woke the parked waiter");
    int wst = -1;
    EXPECT(arena_thread_join(&waiter, &wst) == 0 && wst == 42, "timed wait then notify observed");

    /* Detach: the thread runs to exit under the kernel; join is then stale. */
    arena_thread_t detached;
    EXPECT(arena_thread_create(&detached, quiet_worker, NULL) == 0, "detach create");
    EXPECT(arena_thread_detach(&detached) == 0, "detach");
    int dst = 0;
    EXPECT(arena_thread_join(&detached, &dst) == ARENA_E_STALE, "join after detach is stale");
    EXPECT(arena_thread_join(&workers[0], &dst) == ARENA_E_STALE, "second join is stale");

    /* Stale kernel handle: copy a mutex's key, destroy it, then use the copy. */
    arena_mutex_t stale_mutex;
    EXPECT(arena_mutex_init(&stale_mutex) == 0, "stale mutex init");
    arena_mutex_t stale_copy = stale_mutex;
    EXPECT(arena_mutex_check(&stale_copy) == 0, "live key validates");
    EXPECT(arena_mutex_destroy(&stale_mutex) == 0, "destroy");
    /* The copy still looks initialized locally; the kernel must refuse it. */
    EXPECT(arena_mutex_check(&stale_copy) == ARENA_STATUS_BAD_ARG, "stale key rejected by kernel");

    /* Key exhaustion: the domain refuses keys past its quota. */
    static arena_mutex_t many[40];
    unsigned made = 0;
    int refused = 0;
    for (unsigned i = 0; i < 40; i++) {
        int rc = arena_mutex_init(&many[i]);
        if (rc == 0) {
            made++;
        } else {
            refused = rc;
            break;
        }
    }
    EXPECT(refused == ARENA_STATUS_QUOTA, "key exhaustion refused with QUOTA");
    EXPECT(made >= 1 && made < 40, "some keys minted before refusal");
    uint64_t keys_now = 0;
    /* Live keys: counter, once, gate mutex, gate condvar, plus the minted ones. */
    EXPECT(arena_sync_key_count(&keys_now) == 0 && keys_now == keys_base + 4 + made,
           "key accounting exact");
    for (unsigned i = 0; i < made; i++) {
        (void)arena_mutex_destroy(&many[i]);
    }
    EXPECT(arena_mutex_destroy(&counter_lock) == 0, "counter mutex destroy");
    EXPECT(arena_condvar_destroy(&gate_cv) == 0 && arena_mutex_destroy(&gate_lock) == 0,
           "gate destroy");
    EXPECT(arena_once_destroy(&once_gate) == 0, "once destroy");
    uint64_t keys_end = 0;
    EXPECT(arena_sync_key_count(&keys_end) == 0 && keys_end == keys_base, "keys return to baseline");
    group_result("T9 threads-sync", failed_checks == before, checks - n0);
}

/* ------------------------------------------------------------- main ------- */
int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    arena_printf("[c-native] C application entered through the ARST v2 startup gate\n");
    t1_startup();
    t2_streams();
    t3_stdin();
    t4_rings();
    t5_allocator();
    t6_time();
    t7_logic();
    t8_tls();
    t9_sync();
    /* The kernel records the process status from the LAST thread to exit
     * (syscall.rs record_process_status_if_last). The detached quiet worker
     * may still be exiting, so wait (bounded, ~4 s) until main is the only
     * live thread; main's defined status is then the recorded one. A worker
     * that never exits fails the run instead of hanging or passing silently. */
    for (int i = 0; i < 4000 && arena_thread_count() > 1; i++) {
        (void)arena_busy_sleep_us(1000);
    }
    EXPECT(arena_thread_count() == 1, "detached worker reaped before main exits");
    int pass = groups_passed == 9 && failed_checks == 0;
    arena_printf("[c-native] RESULT %s groups=%d/%d checks=%d failed=%d\n",
                 pass ? "PASS" : "FAIL", groups_passed, 9, checks, failed_checks);
    /* Close the granted output endpoints so the desktop sees EOF after it
     * drains, then end with the defined status. */
    (void)arena_stream_close(1);
    (void)arena_stream_close(2);
    return pass ? EXIT_PASS : EXIT_FAIL;
}
