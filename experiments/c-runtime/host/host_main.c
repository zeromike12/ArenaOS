/* Host-test glue: the one runtime service the host build must supply. */
#include <stddef.h>
#include <unistd.h>

#include "arena/rt.h"

long arena_write(int fd, const void *buf, size_t len) {
    if (fd != 1 && fd != 2) {
        return -1;
    }
    ssize_t n = write(fd, buf, len);
    return (long)n;
}

/* Host stub for the ARST record: the host build has no startup grant. The
 * ring-only tests never attach stdio, so NULL is the correct answer here. */
const arena_startup_t *arena_startup(void) {
    return NULL;
}
