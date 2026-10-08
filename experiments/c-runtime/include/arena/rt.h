/*
 * ArenaOS minimal C application runtime - EXPERIMENTAL (C1).
 *
 * Supported contract (docs/compat/C1-INTEGRATION-CONTRACT.md):
 *   - entry: _start -> ARST v2 startup gate (slot 0, verified against the live
 *     capability table) -> stack -> TLS -> .init_array -> main(argc, argv)
 *   - argv/envp come only from the verified startup record
 *   - stdio: fd 0/1/2 are LOCAL indices onto the StandardStreamSet/stream-wake
 *     capabilities granted by the startup record; they are not authority
 *   - memory: hardened buddy allocator over one process-owned VM reservation
 *   - threads/sync: SYS_THREAD_* and SYS_SYNC_* over granted SyncDomain
 *   - time: monotonic microseconds (SYS_CLOCK_NOW)
 *
 * Legacy only: SYS_DEBUG_WRITE. It is reachable solely through the explicit
 * opt-in arena_legacy_serial_enable(), which the boot probe calls. Ordinary
 * C applications never reach it, and a C app without granted streams fails
 * its stdio calls rather than falling back to serial.
 *
 * Names: the arena_ prefix keeps this runtime from shadowing a host libc.
 * The guest build also exports the standard memory and string names
 * (string.c) because compilers emit calls to them.
 */
#ifndef ARENA_RT_H
#define ARENA_RT_H

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"

#define ARENA_NORETURN __attribute__((noreturn))

/* Runtime-local error codes. Kernel statuses stay in the ARENA_STATUS_* set
 * above; these never collide with them (all are below -1000). */
#define ARENA_E_NO_STARTUP (-1001)  /* no ARST record was granted (slot 0 empty) */
#define ARENA_E_STARTUP (-1002)     /* ARST record present but refused */
#define ARENA_E_NO_CAP (-1003)      /* a required granted capability is absent */
#define ARENA_E_NO_STREAMS (-1004)  /* stdio requested without a stream grant */
#define ARENA_E_WOULD_BLOCK (-1005) /* ring full (write) or empty and open (read) */
#define ARENA_E_BROKEN_PIPE (-1006) /* peer reader closed */
#define ARENA_E_CLOSED (-1007)      /* this endpoint already closed */
#define ARENA_E_CORRUPT (-1008)     /* ring indices violate the protocol */
#define ARENA_E_INVALID (-1009)     /* bad argument, fd, or pointer */
#define ARENA_E_STALE (-1010)       /* handle already joined, detached, or freed */
#define ARENA_E_NOMEM (-1011)       /* allocation or VM commit refused */
#define ARENA_E_NO_WAITER (-1012)   /* blocking call without a granted notification */
#define ARENA_E_OVERFLOW (-1013)    /* size arithmetic would overflow */
#define ARENA_E_UNSUPPORTED (-1014) /* the record or runtime does not provide this */

/* --- process lifecycle --------------------------------------------------- */

/* Provided by the application. argv/envp come from the verified startup
 * record when one was granted; otherwise argc = 0 and argv = NULL. */
int main(int argc, char **argv);

/* Terminate the calling thread; the kernel ends the process when its last
 * thread exits and records `status` as the process exit status. */
ARENA_NORETURN void arena_exit(int status);

/* --- startup ABI v2 ------------------------------------------------------ */

typedef struct arena_cap_desc {
    uint16_t slot;
    uint8_t role;
    uint8_t kind;
    uint32_t rights;
} arena_cap_desc_t;

typedef struct arena_startup {
    int present; /* 1: a verified ARST v2 record; 0: none granted */
    uint32_t flags;
    uint16_t instance_slot;
    uint64_t instance_generation;
    int argc;
    char **argv;
    int envc;
    char **envp;
    unsigned cap_count;
    arena_cap_desc_t caps[ARENA_ARST_CAPABILITY_MAX];
    unsigned stream_set;   /* cap index of StandardStreamSet, or ARENA_ARST_NONE */
    unsigned stream_wake;  /* cap index of StreamWake, or ARENA_ARST_NONE */
    unsigned sync_domain;  /* cap index of SyncDomain, or ARENA_ARST_NONE */
    unsigned notification; /* cap index of a private RW Notification, or NONE */
} arena_startup_t;

/* Verified record (present == 1) after crt start, else present == 0. */
const arena_startup_t *arena_startup(void);

/* EXPERIMENTAL (C2.4): number of verified descriptors with this kind whose rights
 * include rights_mask; -1 when no verified record is present. Counts only; it does
 * not identify a service (no role exists for graphical services in Startup ABI v2). */
int arena_startup_count_kind(const arena_startup_t *s, uint8_t kind, uint32_t rights_mask);

/* --- output ------------------------------------------------------------- */

/* Explicit opt-in to the legacy SYS_DEBUG_WRITE sink. Only the boot probe
 * calls this. Never call it from an ordinary application. */
void arena_legacy_serial_enable(void);

/* printf sink: fd 1/2 -> granted stdout/stderr stream (blocking, all bytes),
 * or the legacy serial sink when explicitly enabled. Returns bytes accepted
 * or a negative ARENA_E_* / ARENA_STATUS_* value. */
long arena_write(int fd, const void *buf, size_t len);
int arena_printf(const char *fmt, ...);
int arena_vprintf(const char *fmt, va_list ap);
int arena_snprintf(char *buf, size_t n, const char *fmt, ...);
int arena_vsnprintf(char *buf, size_t n, const char *fmt, va_list ap);

/* --- granted standard streams (ADR-0104 ring protocol) ------------------- */

/* Attach the granted StandardStreamSet and wake capability. Returns 0,
 * ARENA_E_NO_STREAMS when the record grants none, or ARENA_E_STARTUP. */
int arena_stdio_init(void);

/* Partial transfer. fd 1 or 2 for writes, fd 0 for reads. A short count is
 * valid; a full ring returns ARENA_E_WOULD_BLOCK. A read returns 0 only at
 * EOF (writer closed and drained). */
long arena_stream_write_some(int fd, const void *buf, size_t len);
long arena_stream_read_some(int fd, void *buf, size_t len);

/* Blocking forms: write until every byte is accepted; read until at least
 * one byte or EOF. They wait on the granted private Notification
 * (ARENA_E_NO_WAITER if none was granted). Backpressure and wakeups use the
 * desktop's BADGE_STREAM_READY and the wake notification's STREAM badge. */
long arena_stream_write(int fd, const void *buf, size_t len);
long arena_stream_read(int fd, void *buf, size_t len);

/* Close this application's endpoint: writer close for fd 1/2 (the peer sees
 * EOF after draining), reader close for fd 0 (peer writes get BROKEN_PIPE). */
int arena_stream_close(int fd);

/* Protocol over any ASTR page: the granted mapping, or a private test page.
 * The page must be 4096 bytes and 4096-aligned. channel: 0 stdin, 1 stdout,
 * 2 stderr. */
int arena_ring_page_init(void *page);
int arena_ring_page_check(const void *page);
long arena_ring_write_some(void *page, unsigned channel, const void *buf, size_t len);
long arena_ring_read_some(void *page, unsigned channel, void *buf, size_t len);
int arena_ring_writer_closed(const void *page, unsigned channel);
void arena_ring_close_writer(void *page, unsigned channel);
void arena_ring_close_reader(void *page, unsigned channel);

/* --- time --------------------------------------------------------------- */

/* Monotonic microseconds since boot (kernel timekeeping). */
uint64_t arena_clock_us(void);
/* Voluntarily reschedule the calling thread. */
void arena_yield(void);
/* Sleep by yielding until at least `us` monotonic microseconds elapsed.
 * Busy: it burns CPU. Returns 0. */
int arena_busy_sleep_us(uint64_t us);

/* --- memory ------------------------------------------------------------- */

void *arena_malloc(size_t n);
void *arena_calloc(size_t count, size_t size);
void *arena_realloc(void *p, size_t n);
void arena_free(void *p);
/* Aligned allocation (C2.2): align a power of two. align <= 16 is plain malloc;
 * 32 <= align <= 64 KiB is served from the same buddy heap. NULL on refusal. */
void *arena_aligned_alloc(size_t align, size_t n);

struct arena_heap_stats {
    uint64_t reserved_pages;  /* VM region capacity */
    uint64_t committed_pages; /* pages committed so far */
    uint64_t live_blocks;     /* currently allocated blocks */
    uint64_t live_bytes;      /* sum of live block capacities */
    uint64_t alloc_calls;
    uint64_t free_calls;
    uint64_t reuse_hits;      /* allocations served from a previously used block */
    uint64_t refused;         /* allocations that returned NULL */
    uint64_t bad_frees;       /* free/realloc of a pointer not live (ignored) */
};
void arena_heap_stats(struct arena_heap_stats *out);

/* --- threads ------------------------------------------------------------- */

typedef int (*arena_thread_fn)(void *arg);

/* Per-thread handle. Created by arena_thread_create; consumed by join or
 * detach. The stack region is owned by the runtime until join. */
typedef struct arena_thread {
    uint64_t id;
    uint64_t region_slot;
    uintptr_t region_base;
    uint32_t state; /* 0 empty, 1 running (joinable), 2 detached, 3 joined */
} arena_thread_t;

/* Returns 0, or a negative status (ARENA_STATUS_QUOTA when the process
 * already has MAX_USER_THREADS_PER_PROCESS live user threads). */
int arena_thread_create(arena_thread_t *t, arena_thread_fn fn, void *arg);
int arena_thread_join(arena_thread_t *t, int *status_out);
int arena_thread_detach(arena_thread_t *t);
ARENA_NORETURN void arena_thread_exit(int status);
int arena_thread_count(void);

/* --- synchronization (SyncDomain-backed, ADR-0107) ----------------------- */

typedef struct arena_mutex {
    uint32_t locked;
    uint32_t initialized;
    uint64_t token;
} arena_mutex_t;

typedef struct arena_condvar {
    uint32_t initialized;
    uint64_t token;
} arena_condvar_t;

typedef struct arena_once {
    uint32_t state; /* 0 idle, 1 running, 2 complete */
    uint32_t initialized;
    uint64_t token;
} arena_once_t;

/* Bind the granted SyncDomain. Required before any *_init. */
int arena_sync_init(void);
int arena_mutex_init(arena_mutex_t *m);
int arena_mutex_destroy(arena_mutex_t *m);
int arena_mutex_lock(arena_mutex_t *m);
/* 0 acquired, 1 busy, negative error. */
int arena_mutex_trylock(arena_mutex_t *m);
/* 0 when the kernel wait key is live; negative (BAD_ARG when stale). */
int arena_mutex_check(const arena_mutex_t *m);
int arena_mutex_unlock(arena_mutex_t *m);
int arena_condvar_init(arena_condvar_t *c);
int arena_condvar_destroy(arena_condvar_t *c);
/* Atomically release m, wait for a notify, and re-acquire m. */
int arena_condvar_wait(arena_condvar_t *c, arena_mutex_t *m);
/* As wait; *timed_out is set when the wait ended by timeout. */
int arena_condvar_wait_timeout(arena_condvar_t *c, arena_mutex_t *m, uint64_t timeout_us,
                               int *timed_out);
long arena_condvar_notify_one(arena_condvar_t *c);
long arena_condvar_notify_all(arena_condvar_t *c);
int arena_once_call(arena_once_t *o, void (*init)(void));
int arena_once_init(arena_once_t *o);
int arena_once_destroy(arena_once_t *o);
int arena_sync_key_count(uint64_t *keys_out);

/* --- string/memory (src/string.c) and runtime facts ---------------------- */

/* Bounds of the VM stack the runtime switched to before main(). */
void arena_stack_bounds(uintptr_t *lo, uintptr_t *hi);

/* Non-zero once the main thread's TLS block is installed in FS.base. */
int arena_tls_ready(void);

#endif /* ARENA_RT_H */
