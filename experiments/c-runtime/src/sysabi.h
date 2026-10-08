/*
 * Internal kernel-facing constants and the raw syscall stub (C2.1).
 *
 * Moved out of the public arena/abi.h so that public headers do not expose
 * kernel calling details. Not installed in the SDK. Checked against the Rust
 * mirror by `run.py abi` (which reads this file as well as arena/abi.h).
 */
#ifndef ARENA_SYSABI_INTERNAL_H
#define ARENA_SYSABI_INTERNAL_H

#include <stdint.h>

/* --- syscall numbers (userspace/abi.rs) ----------------------------------- */
#define ARENA_SYS_DEBUG_WRITE 1u          /* legacy diagnostics; NOT stdio */
#define ARENA_SYS_THREAD_EXIT 2u
#define ARENA_SYS_TRY_WAIT 32u             /* abi.rs SYS_TRY_WAIT */
#define ARENA_SYS_NOTIFY 10u
#define ARENA_SYS_WAIT 11u
#define ARENA_SYS_CAP_DESTROY 20u
#define ARENA_SYS_CLOCK_NOW 26u
#define ARENA_SYS_TIMER_ARM 27u            /* abi.rs SYS_TIMER_ARM */
#define ARENA_SYS_TIMER_CANCEL 28u         /* abi.rs SYS_TIMER_CANCEL */
#define ARENA_SYS_CAP_DESCRIBE 29u
#define ARENA_SYS_SHARED_PAGES 52u
#define ARENA_SYS_TLS_SET 53u
#define ARENA_SYS_CAP_OCCUPIED 54u
#define ARENA_SYS_SHARED_CREATE 36u        /* abi.rs SYS_SHARED_CREATE */
#define ARENA_SYS_SHARED_MAP 37u
#define ARENA_SYS_SHARED_PHYS 38u          /* abi.rs SYS_SHARED_PHYS */
#define ARENA_SYS_SHARED_INFO 39u          /* abi.rs SYS_SHARED_INFO */
#define ARENA_SYS_SHARED_UNMAP 40u
#define ARENA_SYS_VM_RESERVE 56u
#define ARENA_SYS_VM_COMMIT 57u
#define ARENA_SYS_VM_PROTECT 58u
#define ARENA_SYS_VM_RELEASE 59u
#define ARENA_SYS_VM_QUERY 60u
#define ARENA_SYS_THREAD_YIELD 62u
#define ARENA_SYS_NOTIFICATION_CREATE 63u /* abi.rs SYS_NOTIFICATION_CREATE */
#define ARENA_SYS_THREAD_CREATE 64u
#define ARENA_SYS_THREAD_JOIN 65u
#define ARENA_SYS_THREAD_DETACH 66u
#define ARENA_SYS_THREAD_COUNT 67u
#define ARENA_SYS_SYNC_DOMAIN_CREATE 68u  /* abi.rs SYS_SYNC_DOMAIN_CREATE */
#define ARENA_SYS_SYNC_KEY_CREATE 69u
#define ARENA_SYS_SYNC_KEY_DESTROY 70u
#define ARENA_SYS_SYNC_SEQUENCE 71u
#define ARENA_SYS_SYNC_WAIT 72u
#define ARENA_SYS_SYNC_WAKE 73u
#define ARENA_SYS_SYNC_INFO 74u

/* Kernel SYS_DEBUG_WRITE per-call cap (syscall.rs WRITE_MAX). */
#define ARENA_WRITE_MAX 256u

static inline int64_t arena_syscall6(uint64_t nr, uint64_t a0, uint64_t a1,
                                     uint64_t a2, uint64_t a3, uint64_t a4,
                                     uint64_t a5) {
    uint64_t rax = nr;
    register uint64_t r10 __asm__("r10") = a3;
    register uint64_t r8 __asm__("r8") = a4;
    register uint64_t r9 __asm__("r9") = a5;
    __asm__ volatile("syscall"
                     : "+a"(rax), "+D"(a0), "+S"(a1), "+d"(a2), "+r"(r10),
                       "+r"(r8), "+r"(r9)
                     :
                     : "rcx", "r11", "memory");
    return (int64_t)rax;
}


#endif /* ARENA_SYSABI_INTERNAL_H */
