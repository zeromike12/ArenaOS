/*
 * ArenaOS C2 C11 threads subset (EXPERIMENTAL). Scope: docs/compat/C2-LIBC-COVERAGE.md.
 *
 * Thin bindings over SYS_THREAD_* and SYS_SYNC_* (C1 threads.c / sync.c). Not
 * pthreads. Supported: thrd_create/join/detach/exit/current/equal/yield/sleep,
 * mtx_plain mutexes (lock, trylock, unlock), condition variables with
 * cnd_timedwait on the MONOTONIC clock, call_once, and thread-local storage
 * (_Thread_local, compiler-provided). Refused with thrd_error: recursive and timed
 * mutexes (mtx_timedlock), tss_* keys, and any attribute not listed here.
 * The per-process user-thread quota (MAX_USER_THREADS_PER_PROCESS) applies.
 */
#ifndef ARENA_LIBC_THREADS_H
#define ARENA_LIBC_THREADS_H

#include <stdint.h>
#include "time.h"

#define thread_local _Thread_local
#define ONCE_FLAG_INIT {0}
#define TSS_DTOR_ITERATIONS 0

enum {
    thrd_success = 0,
    thrd_busy = 1,
    thrd_error = 2,
    thrd_nomem = 3,
    thrd_timedout = 4
};

enum {
    mtx_plain = 0,
    mtx_timed = 1,
    mtx_recursive = 2
};

typedef struct arena_thread_record *thrd_t;
typedef int (*thrd_start_t)(void *);

typedef struct arena_mutex_c11 {
    uint32_t locked;
    uint32_t initialized;
    uint64_t token;
    int type;
} mtx_t;

typedef struct arena_condvar_c11 {
    uint32_t initialized;
    uint64_t token;
} cnd_t;

typedef struct arena_once_c11 {
    uint32_t state;
    uint32_t initialized;
    uint64_t token;
} once_flag;

typedef unsigned int tss_t;
typedef void (*tss_dtor_t)(void *);

int thrd_create(thrd_t *thr, thrd_start_t fn, void *arg);
int thrd_join(thrd_t thr, int *res);
int thrd_detach(thrd_t thr);
__attribute__((noreturn)) void thrd_exit(int res);
thrd_t thrd_current(void);
int thrd_equal(thrd_t a, thrd_t b);
int thrd_sleep(const struct timespec *dur, struct timespec *rem);
void thrd_yield(void);

int mtx_init(mtx_t *m, int type);
void mtx_destroy(mtx_t *m);
int mtx_lock(mtx_t *m);
int mtx_trylock(mtx_t *m);
int mtx_unlock(mtx_t *m);
int mtx_timedlock(mtx_t *m, const struct timespec *ts);

int cnd_init(cnd_t *c);
void cnd_destroy(cnd_t *c);
int cnd_wait(cnd_t *c, mtx_t *m);
int cnd_timedwait(cnd_t *c, mtx_t *m, const struct timespec *ts);
int cnd_signal(cnd_t *c);
int cnd_broadcast(cnd_t *c);

void call_once(once_flag *flag, void (*fn)(void));

int tss_create(tss_t *key, tss_dtor_t dtor);
void tss_delete(tss_t key);
void *tss_get(tss_t key);
int tss_set(tss_t key, void *val);

#endif /* ARENA_LIBC_THREADS_H */
