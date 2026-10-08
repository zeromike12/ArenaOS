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
