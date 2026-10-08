/* Runtime-internal entry points shared by the C1 translation units. Not part
 * of the application ABI (include/arena/rt.h). */
#ifndef ARENA_INTERNAL_H
#define ARENA_INTERNAL_H

#include <stddef.h>

/* Verify the ARST v2 record (if any) against the live capability table.
 * Runs before the heap and the stack switch. 0, or ARENA_E_STARTUP. */
int arena_startup_gate(void);

/* Legacy SYS_DEBUG_WRITE sink. Reachable only after arena_legacy_serial_enable. */
/* Weak: defined only when legacy_debug.o is linked (boot probe). Ordinary images
 * leave it NULL, so no debug-syscall code is pulled in. */
long arena_legacy_write(int fd, const void *buf, size_t len) __attribute__((weak));

#endif /* ARENA_INTERNAL_H */
