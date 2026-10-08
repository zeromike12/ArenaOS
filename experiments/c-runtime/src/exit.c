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

static void (*handlers[ATEXIT_MAX])(void);
static unsigned handler_count;
static unsigned handlers_running;

int arena_atexit(void (*fn)(void)) {
    if (fn == NULL) {
        return -1;
    }
    unsigned idx = __atomic_fetch_add(&handler_count, 1u, __ATOMIC_ACQ_REL);
    if (idx >= ATEXIT_MAX) {
        __atomic_fetch_sub(&handler_count, 1u, __ATOMIC_ACQ_REL);
        return -1; /* registry full: ENOMEM-class refusal, nothing registered */
    }
    handlers[idx] = fn;
    return 0;
}

void arena_c_run_exit_handlers(void) {
    /* A handler that calls exit() must not re-run the registry. */
    if (__atomic_exchange_n(&handlers_running, 1u, __ATOMIC_ACQ_REL) != 0u) {
        return;
    }
    unsigned n = __atomic_load_n(&handler_count, __ATOMIC_ACQUIRE);
    if (n > ATEXIT_MAX) {
        n = ATEXIT_MAX;
    }
    while (n > 0) {
        n--;
        handlers[n]();
    }
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
