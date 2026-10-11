/*
 * Process exit, atexit, and assert (C2.1 / C2.2).
 *
 * Order at normal exit: main returns -> atexit handlers run in reverse
 * registration order -> arena_exit(status). The status is the value returned
 * by main or passed to exit(). abort() exits with status 134 without running
 * handlers. Registration must finish before exit runs on any thread.
 */
#include <stddef.h>

#include "arena/rt.h"
#include "libc_impl.h"
#include "libc_internal.h"

#define ATEXIT_MAX 32u

/* C2.1.4 registry. A slot is reserved by a bounded CAS on `reserved`, and its
 * pointer is published with a release store. An exiting thread reads `reserved`
 * and then only slots it sees published (non-zero, acquire). So a handler is never
 * called before its slot is initialized, and a reservation that never publishes
 * (a concurrent registration that has not returned) is skipped. Any registration
 * that RETURNED before exit began is published and therefore runs. Slots are
 * uintptr_t because __atomic builtins are defined on integers and object pointers
 * uniformly across GCC and Clang. */
static uintptr_t handlers[ATEXIT_MAX];
static unsigned reserved;               /* number of reserved slots, <= ATEXIT_MAX */
static unsigned handlers_active;        /* 1 while one thread is running the handlers */
static unsigned handlers_done;          /* 1 once the run has completed (never runs again) */
static _Thread_local int in_handlers;   /* 1 on the thread that is running them */

int arena_atexit(void (*fn)(void)) {
    if (fn == NULL) {
        return -1;
    }
    unsigned idx = __atomic_load_n(&reserved, __ATOMIC_ACQUIRE);
    for (;;) {
        if (idx >= ATEXIT_MAX) {
            return -1; /* registry full: nothing reserved, nothing registered */
        }
        if (__atomic_compare_exchange_n(&reserved, &idx, idx + 1u, 0, __ATOMIC_ACQ_REL,
                                        __ATOMIC_ACQUIRE)) {
            break;
        }
    }
    __atomic_store_n(&handlers[idx], (uintptr_t)fn, __ATOMIC_RELEASE);
    return 0;
}

/* Runs the registered handlers, newest first, AT MOST ONCE per process (C2.1.4):
 *   - a handler that calls exit on the same thread returns at once (in_handlers);
 *   - a DIFFERENT thread that reaches exit while the run is in progress waits
 *     (yielding) until it completes, so it never terminates the process mid-handler;
 *   - any caller after the run completed returns at once (handlers_done), so no
 *     handler runs twice. */
void arena_c_run_exit_handlers(void) {
    if (in_handlers) {
        return;
    }
    for (;;) {
        if (__atomic_load_n(&handlers_done, __ATOMIC_ACQUIRE)) {
            return;
        }
        if (__atomic_exchange_n(&handlers_active, 1u, __ATOMIC_ACQ_REL) == 0u) {
            break; /* this thread runs the handlers */
        }
        arena_yield(); /* another thread is running them: wait for handlers_done */
    }
    if (__atomic_load_n(&handlers_done, __ATOMIC_ACQUIRE)) {
        __atomic_store_n(&handlers_active, 0u, __ATOMIC_RELEASE);
        return;
    }
    in_handlers = 1;
    unsigned n = __atomic_load_n(&reserved, __ATOMIC_ACQUIRE);
    while (n > 0) {
        n--;
        uintptr_t fn = __atomic_load_n(&handlers[n], __ATOMIC_ACQUIRE);
        if (fn != 0) {
            ((void (*)(void))fn)();
        }
    }
    in_handlers = 0;
    __atomic_store_n(&handlers_done, 1u, __ATOMIC_RELEASE);
    __atomic_store_n(&handlers_active, 0u, __ATOMIC_RELEASE);
}

/* Normal exit order (C11 7.22.4): atexit handlers, then flush and close the
 * standard streams, then the process ends. The close is what lets the granted
 * output endpoints signal EOF to the broker after the last byte is drained. */
void arena_libc_exit(int status) {
    arena_c_run_exit_handlers();
    (void)arena_fflush(arena_stdout_object());
    (void)arena_fflush(arena_stderr_object());
    (void)arena_stream_close(1);
    (void)arena_stream_close(2);
    arena_exit(status);
}

void arena_libc_Exit(int status) { arena_exit(status); }

void arena_libc_abort(void) { arena_exit(134); }

void __arena_assert_fail(const char *expr, const char *file, int line) {
    (void)arena_fprintf(arena_stderr_object(), "assertion failed: %s (%s:%d)\n", expr, file, line);
    arena_libc_abort();
}
