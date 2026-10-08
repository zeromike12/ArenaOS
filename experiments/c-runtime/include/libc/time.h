/*
 * ArenaOS C2 time subset (EXPERIMENTAL). Only the MONOTONIC clock exists (kernel
 * SYS_CLOCK_NOW). CLOCK_REALTIME, time(), and clock() are refused with errno ENOSYS
 * because the runtime has no granted wall-clock authority. nanosleep is a busy yield.
 */
#ifndef ARENA_LIBC_TIME_H
#define ARENA_LIBC_TIME_H

#include <stdint.h>

typedef int64_t time_t;
typedef int64_t clock_t;
typedef int clockid_t;

struct timespec {
    time_t tv_sec;
    long tv_nsec;
};

#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define TIME_UTC 1 /* maps to CLOCK_MONOTONIC here; NOT wall time (see C2-LIBC-COVERAGE.md) */
#define CLOCKS_PER_SEC 1000000L

int clock_gettime(clockid_t id, struct timespec *ts);
int nanosleep(const struct timespec *req, struct timespec *rem);
time_t time(time_t *out);
clock_t clock(void);

#endif /* ARENA_LIBC_TIME_H */
