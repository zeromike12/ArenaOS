/*
 * Phase 16 Notification-selection guest probe (EXPERIMENTAL).
 *
 * This signed headless C app receives the ordinary Desktop stream launch
 * record. It independently counts C2.1.5 candidates in the verified
 * record (role Other, kind Notification, rights exactly READ|WRITE),
 * reports the runtime's selected index, and proves the consequence:
 *
 *   sole candidate -> selection is bound; exit 75
 *   two candidates -> selection is NONE and a blocking empty-stdin read
 *                     returns ARENA_E_NO_WAITER immediately; exit 76
 *
 * Production Desktop supplies the sole-candidate positive control. The
 * two-candidate record is produced only by guest_notification.py's
 * explicitly restored RED-mutation Desktop fixture.
 */
#include "arena/rt.h"

#define EXIT_NO_STREAMS 58
#define EXIT_PROBE_FAIL 74
#define EXIT_SOLE_OK 75
#define EXIT_AMBIGUOUS_OK 76

static unsigned count_candidates(const arena_startup_t *s, unsigned slots[2]) {
    unsigned candidates = 0;
    slots[0] = 0;
    slots[1] = 0;
    for (unsigned i = 0; i < s->cap_count; i++) {
        const arena_cap_desc_t *cap = &s->caps[i];
        if (cap->role == ARENA_ARST_ROLE_OTHER &&
            cap->kind == ARENA_CAP_KIND_NOTIFICATION &&
            cap->rights == (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE)) {
            if (candidates < 2) {
                slots[candidates] = cap->slot;
            }
            candidates++;
        }
    }
    return candidates;
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;

    if (arena_stdio_init() != 0) {
        return EXIT_NO_STREAMS;
    }

    const arena_startup_t *s = arena_startup();
    if (s == NULL || !s->present) {
        arena_printf("[c-notify] RESULT FAIL reason=no-startup-record\n");
        return EXIT_PROBE_FAIL;
    }

    unsigned candidate_slots[2];
    const unsigned candidates = count_candidates(s, candidate_slots);
    const unsigned selected = s->notification;
    const unsigned selected_slot =
        selected == ARENA_ARST_NONE ? 0u : s->caps[selected].slot;

    char byte = 0;
    const long peek = arena_stream_read_some(0, &byte, 1);
    if (peek != ARENA_E_WOULD_BLOCK) {
        arena_printf(
            "[c-notify] RESULT FAIL reason=stdin-not-empty candidates=%u peek=%ld\n",
            candidates, peek);
        return EXIT_PROBE_FAIL;
    }

    if (candidates == 1 && selected != ARENA_ARST_NONE &&
        selected_slot == candidate_slots[0]) {
        arena_printf("[c-notify] RESULT PASS mode=sole candidates=1 "
                     "selected_index=%u selected_slot=%u peek=%ld\n",
                     selected, selected_slot, peek);
        return EXIT_SOLE_OK;
    }

    if (candidates == 2 && selected == ARENA_ARST_NONE &&
        candidate_slots[0] != 0 && candidate_slots[1] != 0 &&
        candidate_slots[0] != candidate_slots[1]) {
        const long blocking = arena_stream_read(0, &byte, 1);
        if (blocking == ARENA_E_NO_WAITER) {
            arena_printf("[c-notify] RESULT PASS mode=ambiguous candidates=2 "
                         "selected=none slots=%u,%u peek=%ld blocking=%ld\n",
                         candidate_slots[0], candidate_slots[1], peek, blocking);
            return EXIT_AMBIGUOUS_OK;
        }
        arena_printf("[c-notify] RESULT FAIL mode=ambiguous candidates=2 "
                     "selected=none peek=%ld blocking=%ld\n",
                     peek, blocking);
        return EXIT_PROBE_FAIL;
    }

    arena_printf("[c-notify] RESULT FAIL candidates=%u selected_index=%u "
                 "selected_slot=%u slots=%u,%u peek=%ld\n",
                 candidates, selected, selected_slot, candidate_slots[0],
                 candidate_slots[1], peek);
    return EXIT_PROBE_FAIL;
}
