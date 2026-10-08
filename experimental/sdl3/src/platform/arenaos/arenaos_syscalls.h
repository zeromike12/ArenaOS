#ifndef ARENAOS_SYSCALLS_H
#define ARENAOS_SYSCALLS_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define SYS_DEBUG_WRITE   1
#define SYS_THREAD_EXIT   2
#define SYS_IPC_CALL      7
#define SYS_NOTIFY        10
#define SYS_WAIT          11
#define SYS_SPAWN         12
#define SYS_CAP_PHYS      19
#define SYS_CAP_DESTROY   20
#define SYS_CAP_COPY      21
#define SYS_CLOCK_NOW     26
#define SYS_TIMER_ARM     27
#define SYS_TIMER_CANCEL  28
#define SYS_CAP_DESCRIBE  29
#define SYS_SHARED_MAP    37
#define SYS_SHARED_PHYS   38
#define SYS_SHARED_INFO   39
#define SYS_SHARED_UNMAP  40
#define SYS_SHARED_PAGES  52
#define SYS_CAP_OCCUPIED  54

#define CAP_NONE          ((uint64_t)-1)
#define STATUS_BUSY       ((int64_t)-4)

#define SERVICE_ENDPOINT  1
#define SURFACE_SLOT      2
#define CLOCK_SLOT        3
#define DIAGNOSTICS_SLOT  4

static inline int64_t arenaos_syscall1(uint64_t nr, uint64_t a0) {
    int64_t ret;
    asm volatile(
        "syscall"
        : "=a"(ret), "+D"(a0)
        : "0"(nr)
        : "rcx", "r11", "rsi", "rdx", "r8", "r9", "r10", "memory"
    );
    return ret;
}

static inline int64_t arenaos_syscall2(uint64_t nr, uint64_t a0, uint64_t a1) {
    int64_t ret;
    asm volatile(
        "syscall"
        : "=a"(ret), "+D"(a0), "+S"(a1)
        : "0"(nr)
        : "rcx", "r11", "rdx", "r8", "r9", "r10", "memory"
    );
    return ret;
}

static inline int64_t arenaos_syscall3(uint64_t nr, uint64_t a0, uint64_t a1, uint64_t a2) {
    int64_t ret;
    asm volatile(
        "syscall"
        : "=a"(ret), "+D"(a0), "+S"(a1), "+d"(a2)
        : "0"(nr)
        : "rcx", "r11", "r8", "r9", "r10", "memory"
    );
    return ret;
}

static inline int64_t arenaos_syscall6(uint64_t nr, uint64_t a0, uint64_t a1, uint64_t a2, uint64_t a3, uint64_t a4, uint64_t a5) {
    int64_t ret;
    register uint64_t r10 asm("r10") = a3;
    register uint64_t r8  asm("r8")  = a4;
    register uint64_t r9  asm("r9")  = a5;
    asm volatile(
        "syscall"
        : "=a"(ret), "+D"(a0), "+S"(a1), "+d"(a2), "+r"(r10), "+r"(r8), "+r"(r9)
        : "0"(nr)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline void arenaos_exit(int code) {
    arenaos_syscall1(SYS_THREAD_EXIT, (uint64_t)code);
    while (1) {
        asm volatile("pause");
    }
}

static inline void arenaos_debug_write(const char *msg, size_t len) {
    while (len > 0) {
        size_t chunk = (len > 256) ? 256 : len;
        arenaos_syscall2(SYS_DEBUG_WRITE, (uint64_t)msg, (uint64_t)chunk);
        msg += chunk;
        len -= chunk;
    }
}

static inline uint64_t arenaos_clock_now(void) {
    return (uint64_t)arenaos_syscall1(SYS_CLOCK_NOW, 0);
}

#endif /* ARENAOS_SYSCALLS_H */
