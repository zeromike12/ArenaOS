/* Guest runtime services over the native syscall ABI (C1). */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "arena/rt.h"
#include "internal.h"

static int legacy_serial;

void arena_exit(int status) {
    /* Kernel: SYS_THREAD_EXIT diverges for the last thread; the process ends
     * and the status is recorded (syscall.rs sys_thread_exit). */
    (void)arena_syscall6(ARENA_SYS_THREAD_EXIT, (uint64_t)(int64_t)status, 0, 0, 0, 0, 0);
    for (;;) {
        __builtin_trap();
    }
}

void arena_legacy_serial_enable(void) {
    legacy_serial = 1;
}

/* Legacy diagnostics only (SYS_DEBUG_WRITE has no capability check and is not
 * a stream). Used solely after arena_legacy_serial_enable. */
long arena_legacy_write(int fd, const void *buf, size_t len) {
    if ((fd != 1 && fd != 2) || (buf == NULL && len != 0)) {
        return ARENA_E_INVALID;
    }
    const unsigned char *p = (const unsigned char *)buf;
    long total = 0;
    while (len > 0) {
        size_t chunk = len < ARENA_WRITE_MAX ? len : ARENA_WRITE_MAX;
        int64_t rc = arena_syscall6(ARENA_SYS_DEBUG_WRITE, (uint64_t)(uintptr_t)p,
                                    (uint64_t)chunk, 0, 0, 0, 0);
        if (rc <= 0) {
            return total > 0 ? total : (long)rc;
        }
        p += rc;
        len -= (size_t)rc;
        total += (long)rc;
    }
    return total;
}

long arena_write(int fd, const void *buf, size_t len) {
    if (legacy_serial) {
        return arena_legacy_write(fd, buf, len);
    }
    if (fd != 1 && fd != 2) {
        return ARENA_E_INVALID;
    }
    return arena_stream_write(fd, buf, len);
}

uint64_t arena_clock_us(void) {
    int64_t us = arena_syscall6(ARENA_SYS_CLOCK_NOW, 0, 0, 0, 0, 0, 0);
    return us < 0 ? 0 : (uint64_t)us;
}

void arena_yield(void) {
    (void)arena_syscall6(ARENA_SYS_THREAD_YIELD, 0, 0, 0, 0, 0, 0);
}

int arena_busy_sleep_us(uint64_t us) {
    uint64_t start = arena_clock_us();
    while (arena_clock_us() - start < us) {
        arena_yield();
    }
    return 0;
}
