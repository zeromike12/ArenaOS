/*
 * LEGACY diagnostics (C1 boot probe only). SYS_DEBUG_WRITE has no capability check
 * and is not a stream. Kept in its own object so that ordinary applications, which
 * never call arena_legacy_serial_enable, do not link the debug syscall site.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "sysabi.h"
#include "arena/rt.h"
#include "internal.h"

extern int arena_legacy_serial_active;

void arena_legacy_serial_enable(void) {
    arena_legacy_serial_active = 1;
}

/* Legacy diagnostics only (SYS_DEBUG_WRITE has no capability check and is not
 * a stream). Used solely after arena_legacy_serial_enable. */
long arena_legacy_write(int fd, const void *buf, size_t len) {
    if ((fd != 1 && fd != 2) || (buf == NULL && len != 0)) {
        return ARENA_E_INVALID;
    }
    const unsigned char *p = (const unsigned char *)buf;
    long total = 0;
    while (len > 0) {
        size_t chunk = len < ARENA_WRITE_MAX ? len : ARENA_WRITE_MAX;
        int64_t rc = arena_syscall6(ARENA_SYS_DEBUG_WRITE, (uint64_t)(uintptr_t)p,
                                    (uint64_t)chunk, 0, 0, 0, 0);
        if (rc <= 0) {
            return total > 0 ? total : (long)rc;
        }
        p += rc;
        len -= (size_t)rc;
        total += (long)rc;
    }
    return total;
}

