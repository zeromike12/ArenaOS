/* Host-test glue: the one runtime service the host build must supply. */
#include <sched.h>
#include <stddef.h>
#include <string.h>
#include <unistd.h>

#include "arena/abi.h"
#include "arena/rt.h"
#include "sysabi.h"

/* ---- mocked syscall boundary (ARENA_HOSTED only) ------------------------

   Tests program a sequence of mock entries. Each call to arena_syscall6
   pops the next entry and:

   * writes out[0..2] to the caller-provided out[] buffer (a4),
   * copies a reply frame into the caller-provided frame buffer (a5) when
     the entry carries one,
   * returns rax as the kernel result.

   When the programmed sequence is exhausted, the default response is used.
   This drives c_gfx_app.c's production IPC code through controlled
   kernel/service outcomes without executing real Linux syscalls. */

#define ARENA_HOST_MOCK_MAX 64

typedef struct {
    int64_t rax;              /* kernel result (0 = OK, negative = status) */
    uint64_t out0, out1, out2;/* service status word, result, landed cap   */
    uint8_t frame[64];        /* reply frame copied into the caller's buf  */
    int have_frame;           /* 1: overwrite the caller's frame buffer  */
} host_mock_entry_t;

static host_mock_entry_t host_mock_seq[ARENA_HOST_MOCK_MAX];
static unsigned host_mock_len = 0;
static unsigned host_mock_idx = 0;
static host_mock_entry_t host_mock_default = {0, 0, 0, 0, {0}, 0};

void arena_host_mock_push(int64_t rax, uint64_t out0, uint64_t out1,
                          uint64_t out2) {
    if (host_mock_len < ARENA_HOST_MOCK_MAX) {
        host_mock_seq[host_mock_len++] =
            (host_mock_entry_t){rax, out0, out1, out2, {0}, 0};
    }
}

void arena_host_mock_push_frame(int64_t rax, uint64_t out0, uint64_t out1,
                                uint64_t out2, const uint8_t frame[64]) {
    if (host_mock_len < ARENA_HOST_MOCK_MAX) {
        host_mock_entry_t e = {rax, out0, out1, out2, {0}, 1};
        if (frame) memcpy(e.frame, frame, 64);
        host_mock_seq[host_mock_len++] = e;
    }
}

void arena_host_mock_reset(void) {
    host_mock_len = 0;
    host_mock_idx = 0;
    host_mock_default = (host_mock_entry_t){0, 0, 0, 0, {0}, 0};
}

void arena_host_mock_default_set(int64_t rax, uint64_t out0, uint64_t out1,
                                 uint64_t out2) {
    host_mock_default = (host_mock_entry_t){rax, out0, out1, out2, {0}, 0};
}

static void host_mock_apply(const host_mock_entry_t *e, uint64_t nr,
                            uint64_t a1, uint64_t a4, uint64_t a5) {
    /* IPC_CALL (7):  out[] at a4, frame at a5                    */
    /* CAP_DESCRIBE (29): out[] at a1                            */
    if (nr == ARENA_SYS_CAP_DESCRIBE) {
        uint64_t *out = (uint64_t *)(uintptr_t)a1;
        if (out) {
            out[0] = e->out0;
            out[1] = e->out1;
            out[2] = e->out2;
        }
    } else {
        uint64_t *out = (uint64_t *)(uintptr_t)a4;
        uint8_t *frame = (uint8_t *)(uintptr_t)a5;
        if (out) {
            out[0] = e->out0;
            out[1] = e->out1;
            out[2] = e->out2;
        }
        if (e->have_frame && frame) {
            memcpy(frame, e->frame, 64);
        }
    }
}

int64_t arena_syscall6(uint64_t nr, uint64_t a0, uint64_t a1,
                       uint64_t a2, uint64_t a3, uint64_t a4,
                       uint64_t a5) {
    (void)a0; (void)a2; (void)a3;
    const host_mock_entry_t *e;
    if (host_mock_idx < host_mock_len) {
        e = &host_mock_seq[host_mock_idx++];
    } else {
        e = &host_mock_default;
    }
    host_mock_apply(e, nr, a1, a4, a5);
    return e->rax;
}

/* Host test seam for the stdio write path (not part of the runtime API):
 *  - arena_host_write_limit > 0 caps each write to that many bytes (short writes);
 *  - arena_host_write_fail != 0 makes every write fail with that error;
 *  - arena_host_capture (when arena_host_capture_on) records the accepted bytes
 *    instead of writing them to the real descriptor. */
long arena_host_write_limit = 0;
long arena_host_write_fail = 0;
int arena_host_capture_on = 0;
char arena_host_capture[512];
size_t arena_host_capture_n = 0;
long arena_host_write_calls = 0;
/* C2.1.3 seam: total bytes the stream accepts before every later write fails with
 * arena_host_write_fail (or ARENA_E_BROKEN_PIPE if that is 0). -1 = unlimited. This
 * makes an fwrite of several elements fail part-way, through an element. */
long arena_host_write_budget = -1;

long arena_write(int fd, const void *buf, size_t len) {
    if (fd != 1 && fd != 2) {
        return -1;
    }
    arena_host_write_calls++;
    if (arena_host_write_fail != 0) {
        return arena_host_write_fail;
    }
    size_t take = len;
    if (arena_host_write_limit > 0 && take > (size_t)arena_host_write_limit) {
        take = (size_t)arena_host_write_limit;
    }
    if (arena_host_write_budget >= 0) {
        if (arena_host_write_budget == 0) {
            return arena_host_write_fail != 0 ? arena_host_write_fail : ARENA_E_BROKEN_PIPE;
        }
        if (take > (size_t)arena_host_write_budget) {
            take = (size_t)arena_host_write_budget;
        }
        arena_host_write_budget -= (long)take;
    }
    if (arena_host_capture_on) {
        size_t room = sizeof arena_host_capture - arena_host_capture_n;
        size_t keep = take < room ? take : room;
        for (size_t i = 0; i < keep; i++) {
            arena_host_capture[arena_host_capture_n + i] = ((const char *)buf)[i];
        }
        arena_host_capture_n += keep;
        return (long)take;
    }
    ssize_t n = write(fd, buf, take);
    return (long)n;
}

/* Host stub for the ARST record: the host build has no startup grant. The
 * ring-only tests never attach stdio, so NULL is the correct answer here. */
/* Host stub: no startup grant. The guest contract is that arena_startup() never
 * returns NULL; the host returns an empty (present == 0) record the same way. */
const arena_startup_t *arena_startup(void) {
    static const arena_startup_t none;
    return &none;
}

/* Host stand-in for the scheduler yield (the guest binds it to SYS_YIELD). Used by
 * the atexit re-entry path, which yields while another thread runs the handlers. */
/* Host seam for src/libc_time.c (nanosleep's busy wait). src/rt.c, the real binding, talks
 * to the kernel and is not in the host build. No host test sleeps: nanosleep is guest-verified
 * (App A G4). time() and clock() refusals do not reach this seam. */
int arena_busy_sleep_us(uint64_t us) {
    (void)us;
    return 0;
}

void arena_yield(void) {
    sched_yield();
}

/* Host stub for the exit syscall path: the host test never returns from it. */
#include <unistd.h> /* _exit */
void arena_exit(int status) {
    _exit(status);
}
