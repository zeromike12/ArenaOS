/* Runtime-internal entry points shared by the C1 translation units. Not part
 * of the application ABI (include/arena/rt.h). */
#ifndef ARENA_INTERNAL_H
#define ARENA_INTERNAL_H

#include <stddef.h>

#include "arena/rt.h"

/* Verify the ARST v2 record (if any) against the live capability table.
 * Runs before the heap and the stack switch. 0, or ARENA_E_STARTUP. */
int arena_startup_gate(void);

/* C2.1.5: sole wake-notification candidate index, or ARENA_ARST_NONE (startup_query.c). */
unsigned arena_startup_pick_notification(const arena_cap_desc_t *caps, unsigned count);

/* Legacy SYS_DEBUG_WRITE sink. Reachable only after arena_legacy_serial_enable.
 * Phase 16: dispatched through a hook pointer (defined in rt.c, NULL until
 * legacy_debug.c registers it). The previous weak declaration made rt.o carry
 * a reference to arena_legacy_write in every image; Clang kept it as a weak
 * undefined symbol in ordinary executables. rt.o now references nothing from
 * legacy_debug.o, so ordinary images carry no legacy symbol at all. */
typedef long (*arena_legacy_write_fn)(int fd, const void *buf, size_t len);
extern arena_legacy_write_fn arena_legacy_write_hook;

#endif /* ARENA_INTERNAL_H */
