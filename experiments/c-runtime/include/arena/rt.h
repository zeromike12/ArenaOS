/*
 * ArenaOS minimal C application runtime - PROTOTYPE (experimental).
 *
 * Scope (deliberately small; see docs/compat/03-c-runtime-prototype.md):
 *   - entry: _start -> stack switch -> TLS -> .init_array -> main -> exit
 *   - termination: arena_exit (SYS_THREAD_EXIT)
 *   - output: fd 1/2 -> SYS_DEBUG_WRITE (serial; a temporary diagnostics
 *     backdoor in the kernel, NOT a stream ABI)
 *   - memory: size-class allocator over process-owned VM regions
 *     (SYS_VM_RESERVE / SYS_VM_COMMIT)
 *   - time: monotonic microseconds (SYS_CLOCK_NOW); busy-yield sleep
 *   - string/memory primitives and a bounded printf subset
 *
 * Not provided: argv/env (no ARST v2 parser yet), blocking timed waits (need
 * a Notification capability the prototype does not hold), streams, files,
 * signals, threads, locale, floating point (the target has no FPU/SSE
 * contract; see the ABI audit).
 *
 * Names: the arena_ prefix keeps this prototype from silently shadowing a
 * host libc. The guest build also exports the standard memory and string names
 * (string.c) because compilers emit calls to them.
 */
#ifndef ARENA_RT_H
#define ARENA_RT_H

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#define ARENA_NORETURN __attribute__((noreturn))

/* --- process lifecycle ------------------------------------------------- */

/* Provided by the application. Called with argc = 0, argv = NULL (no ARST
 * startup-record parser in the prototype). */
int main(int argc, char **argv);

/* Terminate the calling thread; the kernel ends the process when its last
 * thread exits and records `status` as the process exit status. */
ARENA_NORETURN void arena_exit(int status);

/* --- output ------------------------------------------------------------- */

/* Write up to len bytes to fd 1 or 2 (serial console). Returns bytes
 * accepted, or a negative value on error. Chunks at ARENA_WRITE_MAX. */
long arena_write(int fd, const void *buf, size_t len);
int arena_printf(const char *fmt, ...);
int arena_vprintf(const char *fmt, va_list ap);
int arena_snprintf(char *buf, size_t n, const char *fmt, ...);
int arena_vsnprintf(char *buf, size_t n, const char *fmt, va_list ap);

/* --- time --------------------------------------------------------------- */

/* Monotonic microseconds since boot (kernel timekeeping, TSC-calibrated). */
uint64_t arena_clock_us(void);
/* Voluntarily reschedule the calling thread. */
void arena_yield(void);
/* Sleep by yielding until at least `us` monotonic microseconds have
 * elapsed. Burns CPU; it is not a blocking timer. Returns 0. */
int arena_busy_sleep_us(uint64_t us);

/* --- memory ------------------------------------------------------------- */

void *arena_malloc(size_t n);
void *arena_calloc(size_t count, size_t size);
void *arena_realloc(void *p, size_t n);
void arena_free(void *p);

struct arena_heap_stats {
    uint64_t reserved_pages;  /* VM region capacity */
    uint64_t committed_pages; /* pages committed so far */
    uint64_t live_blocks;     /* currently allocated blocks */
    uint64_t live_bytes;      /* sum of live block capacities */
    uint64_t alloc_calls;
    uint64_t free_calls;
    uint64_t reuse_hits;      /* allocations served from a free list */
    uint64_t refused;         /* allocations that returned NULL */
    uint64_t bad_frees;       /* free() of a pointer not live (ignored) */
};
void arena_heap_stats(struct arena_heap_stats *out);

/* --- startup facts ----------------------------------------------------- */

/* Bounds of the 256 KiB VM stack the runtime switched to before main(). */
void arena_stack_bounds(uintptr_t *lo, uintptr_t *hi);

/* --- thread-local storage ---------------------------------------------- */

/* Non-zero once the TLS block is installed in FS.base. */
int arena_tls_ready(void);

#endif /* ARENA_RT_H */
