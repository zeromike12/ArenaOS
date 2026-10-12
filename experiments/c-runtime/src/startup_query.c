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

/* C2.1.5 notification selection (internal; host-testable). Returns the cap index of
 * the sole candidate wake notification, or ARENA_ARST_NONE. A candidate has role
 * OTHER, kind NOTIFICATION, and exactly RIGHT_READ|RIGHT_WRITE. Zero candidates
 * (headless or none granted) and two or more (ambiguous: nothing is guessed) both
 * return NONE. The rule is the single source of truth for startup.c. */
unsigned arena_startup_pick_notification(const arena_cap_desc_t *caps, unsigned count) {
    unsigned candidates = 0;
    unsigned first = ARENA_ARST_NONE;
    for (unsigned i = 0; caps != NULL && i < count; i++) {
        if (caps[i].role == ARENA_ARST_ROLE_OTHER && caps[i].kind == ARENA_CAP_KIND_NOTIFICATION &&
            caps[i].rights == (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE)) {
            if (candidates == 0) {
                first = i;
            }
            candidates++;
        }
    }
    return candidates == 1 ? first : ARENA_ARST_NONE;
}
