/*
 * Profile discovery over a VERIFIED startup record (C2.4). Read-only: counts and
 * kinds, never authority. No slot is assumed to hold a particular service.
 */
#include <stddef.h>

#include "arena/abi.h"
#include "arena/profile.h"

#define KIND_BADGED_ENDPOINT 12u
#define ROLE_STREAM_SET 6u
#define ROLE_STREAM_WAKE 7u

int arena_profile_query(arena_profile_t *out) {
    if (out == NULL) {
        return ARENA_E_INVALID;
    }
    const arena_startup_t *s = arena_startup();
    if (!s->present) {
        return ARENA_E_NO_STARTUP;
    }
    out->headless = (s->flags & ARENA_ARST_FLAG_HEADLESS) != 0;
    out->badged_endpoints = 0;
    out->shared_regions = 0;
    out->notifications = 0;
    for (unsigned i = 0; i < s->cap_count; i++) {
        const arena_cap_desc_t *d = &s->caps[i];
        if (d->kind == KIND_BADGED_ENDPOINT) {
            out->badged_endpoints++;
        } else if (d->kind == ARENA_CAP_KIND_SHARED_REGION && d->role != ROLE_STREAM_SET) {
            out->shared_regions++;
        } else if (d->kind == ARENA_CAP_KIND_NOTIFICATION && d->role != ROLE_STREAM_WAKE) {
            out->notifications++;
        }
    }
    out->window_ambiguous = out->badged_endpoints > 1;
    return 0;
}

/* The record does not say which BadgedEndpoint is the window service (BLK-C2-01).
 * Refused always. Do not replace this with a slot-number convention. */
int arena_window_endpoint(unsigned *slot_out) {
    (void)slot_out;
    return ARENA_E_UNSUPPORTED;
}
