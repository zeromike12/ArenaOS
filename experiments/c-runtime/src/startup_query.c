/*
 * Read-only queries over a verified startup record (C2.4, EXPERIMENTAL).
 * Pure functions: they never touch the kernel, the capability table, or the
 * broker, and they do not change the Startup ABI v2 wire format.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "arena/rt.h"

/* EXPERIMENTAL (C2.4). Counts verified descriptors of one kind whose rights include
 * rights_mask. This is a necessary check, not service identification: Startup ABI
 * v2 gives graphical endpoints, surfaces, and clocks role Other, which carries no
 * service identity (see docs/compat/C2-FINAL-REPORT.md, blocker B-GFX-1). */
int arena_startup_count_kind(const arena_startup_t *s, uint8_t kind, uint32_t rights_mask) {
    if (s == NULL || !s->present || s->cap_count > ARENA_ARST_CAPABILITY_MAX) {
        return -1;
    }
    int n = 0;
    for (unsigned i = 0; i < s->cap_count; i++) {
        if (s->caps[i].kind == kind && (s->caps[i].rights & rights_mask) == rights_mask) {
            n++;
        }
    }
    return n;
}

