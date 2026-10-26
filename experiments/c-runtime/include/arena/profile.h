/*
 * ArenaOS native application profile discovery (EXPERIMENTAL, C2.4).
 *
 * Reports only what a VERIFIED Startup ABI v2 record proves. It never assumes a
 * slot's service from its position. The window endpoint and the surface are NOT
 * identifiable from the record today (docs/compat/C2-INTEGRATION-PLAN.md BLK-C2-01),
 * so arena_window_endpoint() refuses with ARENA_E_UNSUPPORTED. Nothing here grants
 * authority: these are counts and kinds read from the record.
 */
#ifndef ARENA_PROFILE_H
#define ARENA_PROFILE_H

#include "arena/rt.h"


typedef struct arena_profile {
    int headless;               /* 1 when FLAG_HEADLESS is set (headless profile) */
    unsigned badged_endpoints;  /* descriptors of kind BadgedEndpoint */
    unsigned shared_regions;    /* SharedRegion descriptors that are not the stream set */
    unsigned notifications;     /* Notification descriptors that are not the stream wake */
    int window_ambiguous;       /* 1 when more than one BadgedEndpoint is granted */
} arena_profile_t;

/* Fill `out` from the verified record. Returns 0, or ARENA_E_NO_STARTUP when no
 * record was granted. */
int arena_profile_query(arena_profile_t *out);

/* Window-service discovery. Always ARENA_E_UNSUPPORTED in C2 (BLK-C2-01). Arena 1's
 * ADSK-v1 client must not infer its endpoint from a slot number. */
int arena_window_endpoint(unsigned *slot_out);

#endif /* ARENA_PROFILE_H */
