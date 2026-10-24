/*
 * Native user threads (C1.3) over SYS_THREAD_CREATE / JOIN / DETACH / COUNT.
 *
 * Layout per thread (one 16-page VM reservation, same as the Rust runtime):
 *   page 0       : TLS block (variant II, below TP) + TCB at TP = base+2048,
 *                  start context at base+3072. Committed RW.
 *   page 1       : guard. Never committed, so a stack overflow faults.
 *   pages 2..15  : stack, 56 KiB, committed RW/NX. Top = base + 16 pages.
 * fs_base (TP) is 16-aligned inside page 0, which the kernel requires.
 *
 * Ownership: each thread owns exactly its region. Join waits for exit, then
 * releases the region. Detach hands cleanup to the kernel after exit. Handles
 * are checked locally first (state machine), so a stale or repeated join or
 * detach returns ARENA_E_STALE without a syscall.
 *
 * Limits: the kernel refuses more than MAX_USER_THREADS_PER_PROCESS (4) live
 * user threads with ARENA_STATUS_QUOTA. That refusal is returned, not hidden.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "sysabi.h"
#include "arena/rt.h"
#include "arena/string.h"
#include "internal.h"
#include "vm.h"

#define THREAD_REGION_PAGES 16u
#define THREAD_FIRST_STACK_PAGE 2u
#define THREAD_TP_OFFSET 2048u
#define THREAD_CTX_OFFSET 3072u
#define THREAD_STACK_TOP_OFFSET (THREAD_REGION_PAGES * ARENA_PAGE_SIZE)

#define STATE_EMPTY 0u
#define STATE_RUNNING 1u
#define STATE_DETACHED 2u
#define STATE_JOINED 3u

extern const char __arena_tls_image[];
extern const char __arena_tls_filesz[];
extern const char __arena_tls_memsz[];
extern const char __arena_tls_align[];

struct thread_start {
    arena_thread_fn fn;
    void *arg;
};

static void thread_entry(uint64_t ctx_addr) __attribute__((noreturn));
static void thread_entry(uint64_t ctx_addr) {
    struct thread_start ctx;
    arena_memcpy(&ctx, (const void *)(uintptr_t)ctx_addr, sizeof ctx);
    int status = ctx.fn(ctx.arg);
    arena_thread_exit(status);
}

/* Install the TLS block at the top of page 0 and return the thread pointer,
 * or 0 when the layout does not fit. Each thread gets its own copy of .tdata
 * and zeroed .tbss, so __thread variables are isolated per thread. */
static uintptr_t thread_tls_install(uintptr_t base) {
    uintptr_t memsz = (uintptr_t)__arena_tls_memsz;
    uintptr_t filesz = (uintptr_t)__arena_tls_filesz;
    uintptr_t align = (uintptr_t)__arena_tls_align;
    if (align == 0) {
        align = 1;
    }
    if (align > 16 || (16 % align) != 0 || filesz > memsz) {
        return 0;
    }
    uintptr_t tls_round = (memsz + align - 1) / align * align;
    if (tls_round > THREAD_TP_OFFSET) {
        return 0;
    }
    uintptr_t tp = base + THREAD_TP_OFFSET;
    uint8_t *block = (uint8_t *)(tp - tls_round);
    arena_memcpy(block, __arena_tls_image, (size_t)filesz);
    arena_memset(block + filesz, 0, (size_t)(memsz - filesz));
    *(uintptr_t *)tp = tp; /* TCB self-pointer: %fs:0 */
    return tp;
}

int arena_thread_create(arena_thread_t *t, arena_thread_fn fn, void *arg) {
    if (t == NULL || fn == NULL) {
        return ARENA_E_INVALID;
    }
    t->id = 0;
    t->region_slot = 0;
    t->region_base = 0;
    t->state = STATE_EMPTY;
    uint64_t slot = 0;
    uintptr_t base = 0;
    int rc = arena_vm_reserve(THREAD_REGION_PAGES, &slot, &base);
    if (rc != 0) {
        return rc < 0 ? rc : ARENA_E_NOMEM;
    }
    if (arena_vm_commit(slot, 0, 1) != 0 ||
        arena_vm_commit(slot, THREAD_FIRST_STACK_PAGE,
                        THREAD_REGION_PAGES - THREAD_FIRST_STACK_PAGE) != 0) {
        (void)arena_vm_release(slot, base, THREAD_REGION_PAGES);
        return ARENA_E_NOMEM;
    }
    uintptr_t tp = thread_tls_install(base);
    if (tp == 0) {
        (void)arena_vm_release(slot, base, THREAD_REGION_PAGES);
        return ARENA_E_NOMEM;
    }
    struct thread_start *ctx = (struct thread_start *)(uintptr_t)(base + THREAD_CTX_OFFSET);
    ctx->fn = fn;
    ctx->arg = arg;
    int64_t id = arena_syscall6(ARENA_SYS_THREAD_CREATE, (uint64_t)(uintptr_t)thread_entry,
                                base + THREAD_CTX_OFFSET, slot,
                                base + THREAD_FIRST_STACK_PAGE * ARENA_PAGE_SIZE,
                                base + THREAD_STACK_TOP_OFFSET, tp);
    if (id <= 0) {
        (void)arena_vm_release(slot, base, THREAD_REGION_PAGES);
        return id == 0 ? ARENA_E_INVALID : (int)id;
    }
    t->id = (uint64_t)id;
    t->region_slot = slot;
    t->region_base = base;
    t->state = STATE_RUNNING;
    return 0;
}

int arena_thread_join(arena_thread_t *t, int *status_out) {
    if (t == NULL) {
        return ARENA_E_INVALID;
    }
    if (t->state != STATE_RUNNING) {
        return ARENA_E_STALE;
    }
    uint64_t status = 0;
    int64_t rc = arena_syscall6(ARENA_SYS_THREAD_JOIN, t->id, (uint64_t)(uintptr_t)&status, 0, 0, 0, 0);
    if (rc != ARENA_STATUS_OK) {
        return (int)rc;
    }
    t->state = STATE_JOINED;
    if (arena_vm_release(t->region_slot, t->region_base, THREAD_REGION_PAGES) != 0) {
        return ARENA_E_NOMEM;
    }
    if (status_out != NULL) {
        *status_out = (int)(int64_t)status;
    }
    return 0;
}

int arena_thread_detach(arena_thread_t *t) {
    if (t == NULL) {
        return ARENA_E_INVALID;
    }
    if (t->state != STATE_RUNNING) {
        return ARENA_E_STALE;
    }
    int64_t rc = arena_syscall6(ARENA_SYS_THREAD_DETACH, t->id, 0, 0, 0, 0, 0);
    if (rc != ARENA_STATUS_OK) {
        return (int)rc;
    }
    t->state = STATE_DETACHED; /* the kernel releases the stack after exit */
    return 0;
}

void arena_thread_exit(int status) {
    (void)arena_syscall6(ARENA_SYS_THREAD_EXIT, (uint64_t)(int64_t)status, 0, 0, 0, 0, 0);
    for (;;) {
        __builtin_trap();
    }
}

int arena_thread_count(void) {
    /* Non-negative live count (main thread included), or a negative status. */
    return (int)arena_syscall6(ARENA_SYS_THREAD_COUNT, 0, 0, 0, 0, 0, 0);
}
