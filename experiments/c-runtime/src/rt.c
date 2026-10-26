/* Guest runtime services over the native syscall ABI (C1). */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "sysabi.h"
#include "arena/rt.h"
#include "internal.h"

/* Legacy SYS_DEBUG_WRITE dispatch. Both variables are written only by
 * arena_legacy_serial_enable (legacy_debug.c). Ordinary images never call it,
 * so the hook stays NULL and no SYS_DEBUG_WRITE path is linked or referenced. */
int arena_legacy_serial_active;
arena_legacy_write_fn arena_legacy_write_hook;

void arena_exit(int status) {
    /* Kernel: SYS_THREAD_EXIT diverges for the last thread; the process ends
     * and the status is recorded (syscall.rs sys_thread_exit). */
    (void)arena_syscall6(ARENA_SYS_THREAD_EXIT, (uint64_t)(int64_t)status, 0, 0, 0, 0, 0);
    for (;;) {
        __builtin_trap();
    }
}

long arena_write(int fd, const void *buf, size_t len) {
    if (arena_legacy_serial_active && arena_legacy_write_hook != NULL) {
        return arena_legacy_write_hook(fd, buf, len);
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
