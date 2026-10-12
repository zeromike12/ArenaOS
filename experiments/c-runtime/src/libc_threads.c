/*
 * C2.3 C11 threads subset. Thin bindings over SYS_THREAD_* and SYS_SYNC_*
 * (threads.c, sync.c). Not pthreads. Unsupported operations return thrd_error
 * (never a fake success): recursive and timed mutexes, tss_* keys.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/rt.h"
#include "libc/threads.h"
#include "libc_impl.h"
#include "libc_internal.h"

/* The C11 mutex and condition variable are layout-compatible with the runtime's
 * arena_mutex_t / arena_condvar_t. The asserts below pin that at compile time. */
_Static_assert(sizeof(mtx_t) >= sizeof(arena_mutex_t), "mtx_t too small for arena_mutex_t");
_Static_assert(sizeof(cnd_t) == sizeof(arena_condvar_t), "cnd_t layout");
_Static_assert(sizeof(once_flag) == sizeof(arena_once_t), "once_flag layout");

/* C2.1.2 ownership. A record has exactly two references:
 *   - the HANDLE reference, held by the creator until thrd_join or thrd_detach
 *     consumes it (C11: a joined or detached handle is not used again);
 *   - the WORKER reference, held by the new thread from its start until the entry
 *     function returns (or thrd_exit runs).
 * The last reference to drop frees the record. The worker drops its reference
 * after its last use of the record, so the free never races the worker's own
 * access. A detached thread therefore frees its record when it exits (or at
 * detach, if it had already exited), with no leak and no use-after-free. The
 * kernel reclaims the thread's stack; the runtime reclaims only the record. */
struct arena_thread_record {
    arena_thread_t t;     /* runtime thread handle (stack owned by the runtime) */
    thrd_start_t fn;
    void *arg;
    uint32_t refs;        /* handle + worker while the worker runs; atomic */
};

static struct arena_thread_record main_record;               /* the initial thread */
static _Thread_local struct arena_thread_record *current_record;

static void record_release(struct arena_thread_record *r) {
    if (__atomic_sub_fetch(&r->refs, 1u, __ATOMIC_ACQ_REL) == 0u) {
        arena_free(r);
    }
}

static int thread_body(void *ctx) {
    struct arena_thread_record *r = (struct arena_thread_record *)ctx;
    current_record = r;
    thrd_start_t fn = r->fn;
    void *arg = r->arg;
    int res = fn(arg);
    /* Worker reference: last use of r. May free r. Nothing touches r afterwards. */
    current_record = NULL;
    record_release(r);
    return res;
}

static int map_status(int rc) {
    if (rc == 0) return thrd_success;
    if (rc == ARENA_STATUS_QUOTA) return thrd_error;
    return thrd_error;
}

int thrd_create(thrd_t *thr, thrd_start_t fn, void *arg) {
    if (thr == NULL || fn == NULL) {
        return thrd_error;
    }
    struct arena_thread_record *r = (struct arena_thread_record *)arena_malloc(sizeof *r);
    if (r == NULL) {
        return thrd_nomem;
    }
    r->fn = fn;
    r->arg = arg;
    r->refs = 2u; /* handle + worker */
    int rc = arena_thread_create(&r->t, thread_body, r);
    if (rc != 0) {
        arena_free(r); /* no worker was started: the record was never shared */
        return map_status(rc);
    }
    *thr = r;
    return thrd_success;
}

int thrd_join(thrd_t thr, int *res) {
    if (thr == NULL || thr == &main_record) {
        return thrd_error;
    }
    int status = 0;
    int rc = arena_thread_join(&thr->t, &status);
    if (rc != 0) {
        return map_status(rc); /* not joined: the handle is still owned by the caller */
    }
    if (res != NULL) {
        *res = status;
    }
    record_release(thr); /* handle reference; the worker has already dropped its own */
    return thrd_success;
}

/* A refused detach (stale, already detached, or joining) keeps the handle valid.
 * On success the handle reference is dropped here; a worker that is still running
 * keeps its own reference and frees the record when it exits. */
int thrd_detach(thrd_t thr) {
    if (thr == NULL || thr == &main_record) {
        return thrd_error;
    }
    int rc = arena_thread_detach(&thr->t);
    if (rc != 0) {
        return map_status(rc);
    }
    record_release(thr);
    return thrd_success;
}

/* Ends only the calling thread. In a worker it drops the worker reference first
 * (so the record is not leaked) and then makes the thread exit. From the initial
 * thread there is no worker reference; the kernel last-thread rule applies. */
void thrd_exit(int res) {
    struct arena_thread_record *r = current_record;
    if (r != NULL) {
        current_record = NULL;
        record_release(r);
    }
    arena_thread_exit(res);
}

thrd_t thrd_current(void) {
    return current_record != NULL ? current_record : &main_record;
}

int thrd_equal(thrd_t a, thrd_t b) { return a == b; }

int thrd_sleep(const struct timespec *dur, struct timespec *rem) {
    return arena_nanosleep(dur, rem) == 0 ? thrd_success : thrd_error;
}

void thrd_yield(void) { arena_yield(); }

/* C2.1.1: only mtx_plain (exactly 0) is implemented. mtx_timed and mtx_recursive,
 * and any unknown bit, are refused here, at init, so the type never claims a
 * capability the runtime lacks (mtx_timedlock is refused for the same reason). */
int mtx_init(mtx_t *m, int type) {
    if (m == NULL || type != mtx_plain) {
        return thrd_error;
    }
    if (arena_sync_init() != 0) {
        return thrd_error;
    }
    m->type = type;
    return map_status(arena_mutex_init((arena_mutex_t *)m));
}

void mtx_destroy(mtx_t *m) {
    if (m != NULL) {
        (void)arena_mutex_destroy((arena_mutex_t *)m);
    }
}

int mtx_lock(mtx_t *m) {
    if (m == NULL) return thrd_error;
    return map_status(arena_mutex_lock((arena_mutex_t *)m));
}

int mtx_trylock(mtx_t *m) {
    if (m == NULL) return thrd_error;
    int rc = arena_mutex_trylock((arena_mutex_t *)m);
    if (rc == 0) return thrd_success;
    if (rc == 1) return thrd_busy;
    return thrd_error;
}

int mtx_unlock(mtx_t *m) {
    if (m == NULL) return thrd_error;
    return map_status(arena_mutex_unlock((arena_mutex_t *)m));
}

/* Timed lock is not supported (no timeout path in the mutex binding). Refused. */
int mtx_timedlock(mtx_t *m, const struct timespec *ts) {
    (void)m;
    (void)ts;
    return thrd_error;
}

int cnd_init(cnd_t *c) {
    if (c == NULL || arena_sync_init() != 0) return thrd_error;
    return map_status(arena_condvar_init((arena_condvar_t *)c));
}

void cnd_destroy(cnd_t *c) {
    if (c != NULL) {
        (void)arena_condvar_destroy((arena_condvar_t *)c);
    }
}

int cnd_wait(cnd_t *c, mtx_t *m) {
    if (c == NULL || m == NULL) return thrd_error;
    return map_status(arena_condvar_wait((arena_condvar_t *)c, (arena_mutex_t *)m));
}

/* Kernel bound on one sync wait (kernel/kernel/src/sync_domain.rs MAX_TIMEOUT_US).
 * A timeout of 0 means "wait forever" to the kernel, so 0 is never passed. */
#define ARENA_SYNC_MAX_TIMEOUT_US 86400000000ULL

/* The timeout is an ABSOLUTE time on the MONOTONIC clock (TIME_UTC is mapped to
 * it; there is no wall clock). C2.1.1 policy:
 *   - a negative seconds or nanoseconds field is invalid (thrd_error), never a
 *     deadline far in the future by unsigned wrap;
 *   - seconds whose microsecond value would overflow are invalid (thrd_error);
 *   - a deadline already past waits 1 microsecond, then reports thrd_timedout;
 *   - a remaining wait longer than the kernel bound is REFUSED (thrd_error). It is
 *     not clamped: a clamp would return thrd_timedout before the deadline, which
 *     is a false answer. The mutex is not released for a refused call. */
int cnd_timedwait(cnd_t *c, mtx_t *m, const struct timespec *ts) {
    if (c == NULL || m == NULL || ts == NULL) {
        return thrd_error;
    }
    if (ts->tv_sec < 0 || ts->tv_nsec < 0 || ts->tv_nsec >= 1000000000L) {
        return thrd_error;
    }
    uint64_t sec = (uint64_t)ts->tv_sec;
    if (sec > UINT64_MAX / 1000000u - 1u) {
        return thrd_error; /* sec * 1e6 (+ < 1e6) cannot overflow past this point */
    }
    uint64_t now_us = arena_clock_us();
    uint64_t due_us = sec * 1000000u + (uint64_t)ts->tv_nsec / 1000u;
    uint64_t wait_us = 1u;
    if (due_us > now_us) {
        wait_us = due_us - now_us;
        if (wait_us > ARENA_SYNC_MAX_TIMEOUT_US) {
            return thrd_error;
        }
    }
    int timed_out = 0;
    int rc = arena_condvar_wait_timeout((arena_condvar_t *)c, (arena_mutex_t *)m, wait_us, &timed_out);
    if (rc != 0) {
        return map_status(rc);
    }
    return timed_out ? thrd_timedout : thrd_success;
}

int cnd_signal(cnd_t *c) {
    if (c == NULL) return thrd_error;
    return arena_condvar_notify_one((arena_condvar_t *)c) >= 0 ? thrd_success : thrd_error;
}

int cnd_broadcast(cnd_t *c) {
    if (c == NULL) return thrd_error;
    return arena_condvar_notify_all((arena_condvar_t *)c) >= 0 ? thrd_success : thrd_error;
}

/* call_once: the first thread to claim the flag (initialized 0 -> 1) initializes
 * the runtime once-object and publishes it (initialized -> 2). Other threads
 * yield until it is published, then run arena_once_call, which blocks while the
 * initializer runs and returns at once once it has completed. */
void call_once(once_flag *flag, void (*fn)(void)) {
    if (flag == NULL || fn == NULL) {
        return;
    }
    uint32_t expect = 0u;
    if (__atomic_compare_exchange_n(&flag->initialized, &expect, 1u, 0, __ATOMIC_ACQ_REL,
                                    __ATOMIC_ACQUIRE)) {
        if (arena_sync_init() != 0 || arena_once_init((arena_once_t *)flag) != 0) {
            __atomic_store_n(&flag->initialized, 3u, __ATOMIC_RELEASE); /* failed */
            return;
        }
        __atomic_store_n(&flag->initialized, 2u, __ATOMIC_RELEASE);
    }
    while (__atomic_load_n(&flag->initialized, __ATOMIC_ACQUIRE) == 1u) {
        arena_yield();
    }
    if (__atomic_load_n(&flag->initialized, __ATOMIC_ACQUIRE) != 2u) {
        return; /* init failed: the call is a no-op rather than running fn */
    }
    (void)arena_once_call((arena_once_t *)flag, fn);
}

/* Thread-specific keys are not supported in C2 (no per-thread table binding). */
int tss_create(tss_t *key, tss_dtor_t dtor) {
    (void)key;
    (void)dtor;
    return thrd_error;
}

void tss_delete(tss_t key) { (void)key; }

void *tss_get(tss_t key) {
    (void)key;
    return NULL;
}

int tss_set(tss_t key, void *val) {
    (void)key;
    (void)val;
    return thrd_error;
}
