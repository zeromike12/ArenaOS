/*
 * C2.2 time subset. MONOTONIC only (SYS_CLOCK_NOW). Wall-clock requests are
 * refused with ENOSYS: the runtime holds no granted realtime authority.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/rt.h"
#include "libc_impl.h"
#include "libc_internal.h"
#include "sysabi.h"

#define NSEC_PER_SEC 1000000000L

int arena_c_clock_checked(uint64_t *us_out) {
    int64_t us = arena_syscall6(ARENA_SYS_CLOCK_NOW, 0, 0, 0, 0, 0, 0);
    if (us < 0) {
        return (int)us;
    }
    *us_out = (uint64_t)us;
    return 0;
}

int arena_clock_gettime(clockid_t id, struct timespec *ts) {
    if (ts == NULL) {
        arena_c_set_errno(22);
        return -1;
    }
    if (id == CLOCK_REALTIME) {
        arena_c_set_errno(38); /* ENOSYS: no wall-clock grant */
        return -1;
    }
    if (id != CLOCK_MONOTONIC) {
        arena_c_set_errno(22);
        return -1;
    }
    uint64_t us = 0;
    int rc = arena_c_clock_checked(&us);
    if (rc != 0) {
        arena_c_set_errno(arena_c_errno_for(rc));
        return -1;
    }
    ts->tv_sec = (time_t)(us / 1000000u);
    ts->tv_nsec = (long)((us % 1000000u) * 1000u);
    return 0;
}

/* Busy yield until the monotonic clock has advanced by the requested time. There
 * is no blocking timer binding in C2 (F-C2-13): this burns CPU while it waits. */
int arena_nanosleep(const struct timespec *req, struct timespec *rem) {
    if (req == NULL || req->tv_sec < 0 || req->tv_nsec < 0 || req->tv_nsec >= NSEC_PER_SEC) {
        arena_c_set_errno(22);
        return -1;
    }
    uint64_t total_us = (uint64_t)req->tv_sec * 1000000u + (uint64_t)((req->tv_nsec + 999) / 1000);
    (void)arena_busy_sleep_us(total_us);
    if (rem != NULL) {
        rem->tv_sec = 0;
        rem->tv_nsec = 0;
    }
    return 0;
}

time_t arena_time(time_t *out) {
    (void)out;
    arena_c_set_errno(38); /* ENOSYS: no wall-clock grant */
    return (time_t)-1;
}

clock_t arena_clock(void) {
    arena_c_set_errno(38); /* ENOSYS: no CPU-time accounting exposed to apps */
    return (clock_t)-1;
}
