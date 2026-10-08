/* Host-test glue: the one runtime service the host build must supply. */
#include <stddef.h>
#include <unistd.h>

#include "arena/rt.h"

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

/* Host stub for the exit syscall path: the host test never returns from it. */
#include <unistd.h> /* _exit */
void arena_exit(int status) {
    _exit(status);
}
