/*
 * Mutex, condition variable, and one-time init over granted SyncDomain keys
 * (C1.3; ADR-0107). Algorithms follow userspace/arena-runtime/src/sync.rs:
 *   - each primitive owns one generation-checked key in the granted domain;
 *   - waiters snapshot the key sequence, re-check their predicate, then park
 *     with SYS_SYNC_WAIT(domain, key, observed, 0); the kernel compares and
 *     parks atomically, so a wake between check and park is not lost;
 *   - unlock/notify bump the sequence through SYS_SYNC_WAKE.
 * No polling, no ambient handle, no Linux futex.
 *
 * Owner checking: none. As in the Rust runtime, a mutex does not record its
 * owner, so unlocking a mutex you do not hold is a caller bug and is not
 * detected here. Destroyed or stale keys are rejected by the kernel
 * (ARENA_STATUS_BAD_ARG) and surfaced as errors.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "sysabi.h"
#include "arena/rt.h"
#include "internal.h"
#include "vm.h"

#define WAKE_ONE 1u
#define WAKE_ALL 2u

static uint64_t domain_slot;
static int sync_ready;

int arena_sync_init(void) {
    if (sync_ready) {
        return 0;
    }
    const arena_startup_t *s = arena_startup();
    if (!s->present || s->sync_domain == ARENA_ARST_NONE) {
        return ARENA_E_NO_CAP;
    }
    const arena_cap_desc_t *d = &s->caps[s->sync_domain];
    if (d->kind != ARENA_CAP_KIND_SYNC_DOMAIN || d->rights != (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE)) {
        return ARENA_E_STARTUP;
    }
    uint64_t obs[3] = {0, 0, 0};
    if (arena_syscall6(ARENA_SYS_CAP_DESCRIBE, d->slot, (uint64_t)(uintptr_t)obs, 0, 0, 0, 0) != 0 ||
        obs[0] != ARENA_CAP_KIND_SYNC_DOMAIN || obs[2] != (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE)) {
        return ARENA_E_STARTUP;
    }
    domain_slot = d->slot;
    sync_ready = 1;
    return 0;
}

static int64_t key_create(uint64_t *token) {
    int64_t t = arena_syscall6(ARENA_SYS_SYNC_KEY_CREATE, domain_slot, 0, 0, 0, 0, 0);
    if (t <= 0) {
        return t == 0 ? ARENA_E_INVALID : t;
    }
    *token = (uint64_t)t;
    return 0;
}

static int64_t key_destroy(uint64_t token) {
    return arena_syscall6(ARENA_SYS_SYNC_KEY_DESTROY, domain_slot, token, 0, 0, 0, 0);
}

static int64_t key_sequence(uint64_t token) {
    int64_t seq = arena_syscall6(ARENA_SYS_SYNC_SEQUENCE, domain_slot, token, 0, 0, 0, 0);
    return seq <= 0 ? (seq == 0 ? ARENA_E_INVALID : seq) : seq;
}

static int64_t key_wait(uint64_t token, uint64_t observed, uint64_t timeout_us) {
    return arena_syscall6(ARENA_SYS_SYNC_WAIT, domain_slot, token, observed, timeout_us, 0, 0);
}

static int64_t key_wake(uint64_t token, uint64_t mode) {
    return arena_syscall6(ARENA_SYS_SYNC_WAKE, domain_slot, token, mode, 0, 0, 0);
}

/* --- mutex --------------------------------------------------------------- */

int arena_mutex_init(arena_mutex_t *m) {
    if (m == NULL || !sync_ready) {
        return m == NULL ? ARENA_E_INVALID : ARENA_E_NO_CAP;
    }
    m->locked = 0;
    int64_t rc = key_create(&m->token);
    if (rc != 0) {
        return (int)rc;
    }
    m->initialized = 1;
    return 0;
}

int arena_mutex_destroy(arena_mutex_t *m) {
    if (m == NULL || !m->initialized) {
        return ARENA_E_STALE;
    }
    if (__atomic_load_n(&m->locked, __ATOMIC_ACQUIRE)) {
        return ARENA_STATUS_BUSY;
    }
    int64_t rc = key_destroy(m->token);
    if (rc != 0) {
        return (int)rc;
    }
    m->initialized = 0;
    return 0;
}

static int try_acquire(arena_mutex_t *m) {
    return __atomic_exchange_n(&m->locked, 1u, __ATOMIC_ACQUIRE) == 0;
}

int arena_mutex_trylock(arena_mutex_t *m) {
    if (m == NULL || !m->initialized) {
        return ARENA_E_STALE;
    }
    return try_acquire(m) ? 0 : 1;
}

/* Ask the kernel whether this mutex's wait key is still live in the domain.
 * 0 when live; a negative ARENA_STATUS_* (BAD_ARG for a stale or wrong
 * generation token) otherwise. The lock fast path does not call this. */
int arena_mutex_check(const arena_mutex_t *m) {
    if (m == NULL || !m->initialized) {
        return ARENA_E_STALE;
    }
    int64_t seq = key_sequence(m->token);
    return seq > 0 ? 0 : (int)seq;
}

int arena_mutex_lock(arena_mutex_t *m) {
    if (m == NULL || !m->initialized) {
        return ARENA_E_STALE;
    }
    for (;;) {
        if (try_acquire(m)) {
            return 0;
        }
        int64_t seq = key_sequence(m->token);
        if (seq <= 0) {
            return (int)seq;
        }
        if (try_acquire(m)) {
            return 0;
        }
        int64_t w = key_wait(m->token, (uint64_t)seq, 0);
        if (w == 0) {
            continue;
        }
        if (w == ARENA_STATUS_BUSY) {
            arena_backoff();
            continue;
        }
        return (int)w;
    }
}

int arena_mutex_unlock(arena_mutex_t *m) {
    if (m == NULL || !m->initialized) {
        return ARENA_E_STALE;
    }
    __atomic_store_n(&m->locked, 0u, __ATOMIC_RELEASE);
    int64_t n = key_wake(m->token, WAKE_ONE);
    return n < 0 ? (int)n : 0;
}

/* --- condition variable -------------------------------------------------- */

int arena_condvar_init(arena_condvar_t *c) {
    if (c == NULL || !sync_ready) {
        return c == NULL ? ARENA_E_INVALID : ARENA_E_NO_CAP;
    }
    int64_t rc = key_create(&c->token);
    if (rc != 0) {
        return (int)rc;
    }
    c->initialized = 1;
    return 0;
}

int arena_condvar_destroy(arena_condvar_t *c) {
    if (c == NULL || !c->initialized) {
        return ARENA_E_STALE;
    }
    int64_t rc = key_destroy(c->token);
    if (rc != 0) {
        return (int)rc;
    }
    c->initialized = 0;
    return 0;
}

static int condvar_wait_impl(arena_condvar_t *c, arena_mutex_t *m, uint64_t timeout_us,
                             int *timed_out) {
    if (c == NULL || m == NULL || !c->initialized || !m->initialized) {
        return ARENA_E_STALE;
    }
    int64_t seq = key_sequence(c->token);
    if (seq <= 0) {
        return (int)seq;
    }
    /* Release the mutex, park on the sequence we captured, then re-acquire. */
    __atomic_store_n(&m->locked, 0u, __ATOMIC_RELEASE);
    (void)key_wake(m->token, WAKE_ONE);
    int64_t w = key_wait(c->token, (uint64_t)seq, timeout_us);
    int relock = arena_mutex_lock(m);
    if (timed_out != NULL) {
        *timed_out = 0;
    }
    if (w == ARENA_STATUS_TIMEOUT) {
        if (timed_out != NULL) {
            *timed_out = 1;
        }
        return relock;
    }
    if (relock != 0) {
        return relock;
    }
    return w < 0 ? (int)w : 0;
}

int arena_condvar_wait(arena_condvar_t *c, arena_mutex_t *m) {
    return condvar_wait_impl(c, m, 0, NULL);
}

int arena_condvar_wait_timeout(arena_condvar_t *c, arena_mutex_t *m, uint64_t timeout_us,
                               int *timed_out) {
    return condvar_wait_impl(c, m, timeout_us, timed_out);
}

long arena_condvar_notify_one(arena_condvar_t *c) {
    if (c == NULL || !c->initialized) {
        return ARENA_E_STALE;
    }
    return (long)key_wake(c->token, WAKE_ONE);
}

long arena_condvar_notify_all(arena_condvar_t *c) {
    if (c == NULL || !c->initialized) {
        return ARENA_E_STALE;
    }
    return (long)key_wake(c->token, WAKE_ALL);
}

/* --- once ---------------------------------------------------------------- */

int arena_once_init(arena_once_t *o) {
    if (o == NULL || !sync_ready) {
        return o == NULL ? ARENA_E_INVALID : ARENA_E_NO_CAP;
    }
    o->state = 0;
    int64_t rc = key_create(&o->token);
    if (rc != 0) {
        return (int)rc;
    }
    o->initialized = 1;
    return 0;
}

int arena_once_destroy(arena_once_t *o) {
    if (o == NULL || !o->initialized) {
        return ARENA_E_STALE;
    }
    if (__atomic_load_n(&o->state, __ATOMIC_ACQUIRE) == 1u) {
        return ARENA_STATUS_BUSY;
    }
    int64_t rc = key_destroy(o->token);
    if (rc != 0) {
        return (int)rc;
    }
    o->initialized = 0;
    return 0;
}

int arena_once_call(arena_once_t *o, void (*init)(void)) {
    if (o == NULL || init == NULL || !o->initialized) {
        return ARENA_E_STALE;
    }
    for (;;) {
        uint32_t st = __atomic_load_n(&o->state, __ATOMIC_ACQUIRE);
        if (st == 2u) {
            return 0;
        }
        if (st == 0u) {
            uint32_t expect = 0u;
            if (__atomic_compare_exchange_n(&o->state, &expect, 1u, 0, __ATOMIC_ACQ_REL,
                                            __ATOMIC_ACQUIRE)) {
                init();
                __atomic_store_n(&o->state, 2u, __ATOMIC_RELEASE);
                int64_t n = key_wake(o->token, WAKE_ALL);
                return n < 0 ? (int)n : 0;
            }
            continue;
        }
        int64_t seq = key_sequence(o->token);
        if (seq <= 0) {
            return (int)seq;
        }
        if (__atomic_load_n(&o->state, __ATOMIC_ACQUIRE) != 1u) {
            continue;
        }
        int64_t w = key_wait(o->token, (uint64_t)seq, 0);
        if (w == 0) {
            continue;
        }
        if (w == ARENA_STATUS_BUSY) {
            arena_backoff();
            continue;
        }
        return (int)w;
    }
}

int arena_sync_key_count(uint64_t *keys_out) {
    if (keys_out == NULL || !sync_ready) {
        return ARENA_E_INVALID;
    }
    uint64_t words[4] = {0, 0, 0, 0};
    int64_t rc = arena_syscall6(ARENA_SYS_SYNC_INFO, domain_slot, (uint64_t)(uintptr_t)words, 0, 0, 0, 0);
    if (rc != 0) {
        return (int)rc;
    }
    *keys_out = words[0];
    return 0;
}
