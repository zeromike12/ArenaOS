/*
 * ArenaOS native syscall ABI v1 - C mirror (EXPERIMENTAL, x86_64 only).
 *
 * Source of truth: userspace/abi.rs (userspace mirror) and
 * kernel/kernel/src/arch/x86_64/syscall.rs (kernel dispatcher).
 * experiments/c-runtime/run.py abi fails the audit if any number
 * below disagrees with userspace/abi.rs.
 *
 * Calling convention (ADR-0017): RAX = call number; arguments in RDI, RSI,
 * RDX, R10, R8, R9. Result in RAX as a typed i64: 0 = OK, positive =
 * call-specific payload, negative = typed status. The kernel preserves only
 * RBX, RBP and R12-R15; RCX and R11 are consumed by syscall/sysret; every
 * other register this wrapper names is treated as clobbered.
 *
 * The kernel validates UNUSED trailing argument registers on many calls
 * (e.g. "[a2..a5] == [0;4]"). The wrapper below therefore always passes all
 * six arguments and callers must pass 0 for unused ones.
 */
#ifndef ARENA_ABI_H
#define ARENA_ABI_H

#include <stdint.h>

#define ARENA_SYS_DEBUG_WRITE 1u
#define ARENA_SYS_THREAD_EXIT 2u
#define ARENA_SYS_CLOCK_NOW 26u
#define ARENA_SYS_TLS_SET 53u
#define ARENA_SYS_VM_RESERVE 56u
#define ARENA_SYS_VM_COMMIT 57u
#define ARENA_SYS_VM_PROTECT 58u
#define ARENA_SYS_VM_RELEASE 59u
#define ARENA_SYS_VM_QUERY 60u
#define ARENA_SYS_THREAD_YIELD 62u

#define ARENA_VM_PROT_READ 1u
#define ARENA_VM_PROT_WRITE 2u
#define ARENA_VM_PROT_EXEC 4u

#define ARENA_PAGE_SIZE 4096u
/* Kernel SYS_DEBUG_WRITE per-call cap (syscall.rs WRITE_MAX). */
#define ARENA_WRITE_MAX 256u

#define ARENA_STATUS_OK 0

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

#endif /* ARENA_ABI_H */
