/*
 * c_gfx_refusals.c — V1 guest refusal matrix (qualification client).
 *
 * Runs the *production* client code from c_gfx_app.c — `discover()`, the
 * ADSK framing helpers, `identify_exchange()` and the identity validator —
 * and drives it through the refusal conditions the qualification matrix
 * requires:
 *
 *   1. descriptor reordering (reversed and rotated ARST v2 records);
 *   2. missing and ambiguous endpoint/surface capabilities;
 *   3. wrong rights on either capability;
 *   4. malformed ADSK frames and unsupported versions, refused by the
 *      Desktop's production dispatch;
 *   5. a stale/foreign identity name, refused by the production validator;
 *   6. an unexpected capability transfer on Identify, refused *and disposed*
 *      by the Desktop;
 *   7. surface-layout boundaries, including a real kernel refusal.
 *
 * It is launched exactly like any other signed application: through the
 * Desktop's protected installer and ordinary launch path. Nothing here is
 * privileged, no permission is weakened, and no product syscall is added for
 * testing. It never creates a window, so "no unintended graphical state
 * change" is observable as an unchanged window/instance count.
 *
 * Every case prints ONE line:
 *   [refuse] case=<name> want=<expected> got=<actual> verdict=PASS|FAIL
 * The harness requires every case to be present with verdict=PASS, refuses
 * to accept a missing case, and separately checks that no window appeared.
 */

#define GFX_NO_MAIN 1
/* Every helper in c_gfx_app.c becomes externally visible here: this client
   uses a subset, and an unused `static` would be a build error under the
   project's -Werror. The bodies are the production ones, unchanged. */
#define TEST_EXPORT
#include "c_gfx_app.c"

/* --------------------------------------------------------------- report --- */

static int failures;

static void report(const char *name, long want, long got, int ok) {
    char line[160];
    if (!ok) {
        failures++;
    }
    (void)arena_snprintf(line, sizeof(line),
                         "[refuse] case=%s want=%ld got=%ld verdict=%s",
                         name, want, got, ok ? "PASS" : "FAIL");
    log_line(line);
}

/* One ADSK round-trip that returns the raw kernel result and service status
   word instead of collapsing them, so a *refusal* can be told apart from a
   transport failure. */
static int64_t raw_exchange(const uint8_t request[ADSK_BYTES],
                            uint8_t reply[ADSK_BYTES], uint64_t *status_word) {
    uint64_t out[3];
    out[0] = 0;
    out[1] = 0;
    out[2] = GFX_CAP_NONE;
    arena_memcpy(reply, request, ADSK_BYTES);
    int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, a.endpoint_slot, 0, 0,
                                GFX_CAP_NONE,
                                (uint64_t)(uintptr_t)out,
                                (uint64_t)(uintptr_t)reply);
    *status_word = out[0];
    return rc;
}

/* Build an honest ADSK-v2 Identify request. */
static void build_identify(uint8_t frame[ADSK_BYTES]) {
    arena_memset(frame, 0, ADSK_BYTES);
    arena_memcpy(frame, "ADSK", 4);
    frame[ADSK_OFF_VERSION] = ADSK_V2;
    frame[ADSK_OFF_OPCODE] = ADSK_OP_IDENTIFY;
    frame[8] = (uint8_t)(IDENTIFY_CLIENT_PROTOCOL_VERSION & 0xffu);
    frame[9] = (uint8_t)((IDENTIFY_CLIENT_PROTOCOL_VERSION >> 8) & 0xffu);
}

/* Send a frame the Desktop must refuse. A refusal is the Desktop's typed
   service status word being non-zero while the kernel call itself succeeded
   (the request was delivered and answered — not lost). */
static void expect_wire_refusal(const char *name, uint8_t frame[ADSK_BYTES]) {
    uint8_t reply[ADSK_BYTES];
    uint64_t status_word = 0;
    int64_t rc = raw_exchange(frame, reply, &status_word);
    int refused = (rc == 0) && (status_word != 0);
    report(name, 1, (long)(refused ? 1 : 0), refused);
}

/* ------------------------------------------------------ record fixtures --- */

/* Copy the verified startup record so a fixture can mutate a private copy.
   The client's own authority is untouched: only its view of the record is
   altered, which is exactly the input `discover()` must refuse. */
static void copy_record(arena_startup_t *dst, const arena_startup_t *src) {
    *dst = *src;   /* a private copy: the client's own authority is untouched */
}

static uint64_t discover_rc(const arena_startup_t *record,
                            uint64_t *ep, uint64_t *surf) {
    int rc = discover(record, ep, surf);
    return (uint64_t)(rc < 0 ? 1000 : rc);
}

/* --------------------------------------------------------------- cases --- */

static void case_reordering(const arena_startup_t *s, uint64_t ep, uint64_t surf) {
    arena_startup_t rev, rot;
    copy_record(&rev, s);
    for (unsigned i = 0; i < rev.cap_count / 2u; i++) {
        arena_cap_desc_t t = rev.caps[i];
        rev.caps[i] = rev.caps[rev.cap_count - 1u - i];
        rev.caps[rev.cap_count - 1u - i] = t;
    }
    uint64_t ep_r = 0, surf_r = 0;
    uint64_t rc = discover_rc(&rev, &ep_r, &surf_r);
    report("reorder-reversed", 0, (long)rc, rc == 0 && ep_r == ep && surf_r == surf);

    copy_record(&rot, s);
    if (rot.cap_count > 1u) {
        arena_cap_desc_t first = rot.caps[0];
        for (unsigned i = 0; i + 1u < rot.cap_count; i++) {
            rot.caps[i] = rot.caps[i + 1u];
        }
        rot.caps[rot.cap_count - 1u] = first;
    }
    uint64_t ep_o = 0, surf_o = 0;
    rc = discover_rc(&rot, &ep_o, &surf_o);
    report("reorder-rotated", 0, (long)rc, rc == 0 && ep_o == ep && surf_o == surf);
}

/* Compact the record so every descriptor except index `skip` survives. */
static void without(const arena_startup_t *s, unsigned skip, arena_startup_t *out) {
    *out = *s;
    unsigned n = 0;
    for (unsigned i = 0; i < s->cap_count && i < ARENA_ARST_CAPABILITY_MAX; i++) {
        if (i == skip) {
            continue;
        }
        out->caps[n++] = s->caps[i];
    }
    out->cap_count = n;
}

static int find_kind(const arena_startup_t *s, uint64_t kind) {
    for (unsigned i = 0; i < s->cap_count && i < ARENA_ARST_CAPABILITY_MAX; i++) {
        if (s->caps[i].kind == kind) {
            return (int)i;
        }
    }
    return -1;
}

static void case_discovery_refusals(const arena_startup_t *s) {
    uint64_t ep = 0, surf = 0;
    arena_startup_t t;
    int ep_index = find_kind(s, ARENA_CAP_KIND_BADGED_ENDPOINT);
    int surf_index = find_kind(s, ARENA_CAP_KIND_SHARED_REGION);

    /* missing endpoint -> 2 */
    if (ep_index >= 0) {
        without(s, (unsigned)ep_index, &t);
        uint64_t rc = discover_rc(&t, &ep, &surf);
        report("missing-endpoint", GFX_NO_ENDPOINT, (long)rc,
               rc == (uint64_t)GFX_NO_ENDPOINT);
    } else {
        report("missing-endpoint", GFX_NO_ENDPOINT, -1, 0);
    }

    /* ambiguous endpoint (duplicated) -> 3 */
    if (ep_index >= 0 && s->cap_count + 1u <= ARENA_ARST_CAPABILITY_MAX) {
        copy_record(&t, s);
        t.caps[t.cap_count] = s->caps[ep_index];
        t.cap_count++;
        uint64_t rc = discover_rc(&t, &ep, &surf);
        report("ambiguous-endpoint", GFX_AMBIGUOUS_ENDPOINT, (long)rc,
               rc == (uint64_t)GFX_AMBIGUOUS_ENDPOINT);
    } else {
        report("ambiguous-endpoint", GFX_AMBIGUOUS_ENDPOINT, -1, 0);
    }

    /* wrong endpoint rights (drop COPY) -> 6 */
    if (ep_index >= 0) {
        copy_record(&t, s);
        t.caps[ep_index].rights &= ~(uint32_t)ARENA_RIGHT_COPY;
        uint64_t rc = discover_rc(&t, &ep, &surf);
        report("wrong-endpoint-rights", GFX_WRONG_RIGHTS, (long)rc,
               rc == (uint64_t)GFX_WRONG_RIGHTS);
    } else {
        report("wrong-endpoint-rights", GFX_WRONG_RIGHTS, -1, 0);
    }

    /* missing surface -> 4 */
    if (surf_index >= 0) {
        without(s, (unsigned)surf_index, &t);
        uint64_t rc = discover_rc(&t, &ep, &surf);
        report("missing-surface", GFX_NO_SURFACE, (long)rc,
               rc == (uint64_t)GFX_NO_SURFACE);
    } else {
        report("missing-surface", GFX_NO_SURFACE, -1, 0);
    }

    /* ambiguous surface (duplicated) -> 5 */
    if (surf_index >= 0 && s->cap_count + 1u <= ARENA_ARST_CAPABILITY_MAX) {
        copy_record(&t, s);
        t.caps[t.cap_count] = s->caps[surf_index];
        t.cap_count++;
        uint64_t rc = discover_rc(&t, &ep, &surf);
        report("ambiguous-surface", GFX_AMBIGUOUS_SURFACE, (long)rc,
               rc == (uint64_t)GFX_AMBIGUOUS_SURFACE);
    } else {
        report("ambiguous-surface", GFX_AMBIGUOUS_SURFACE, -1, 0);
    }

    /* wrong surface rights (READ only) -> 6 */
    if (surf_index >= 0) {
        copy_record(&t, s);
        t.caps[surf_index].rights = (uint32_t)ARENA_RIGHT_READ;
        uint64_t rc = discover_rc(&t, &ep, &surf);
        report("wrong-surface-rights", GFX_WRONG_RIGHTS, (long)rc,
               rc == (uint64_t)GFX_WRONG_RIGHTS);
    } else {
        report("wrong-surface-rights", GFX_WRONG_RIGHTS, -1, 0);
    }
}

static void case_wire_refusals(void) {
    uint8_t frame[ADSK_BYTES];

    build_identify(frame);
    frame[3] = 'X';                         /* magic "ADSX" */
    expect_wire_refusal("wire-magic", frame);

    build_identify(frame);
    frame[ADSK_OFF_VERSION] = ADSK_V1;      /* unsupported version for 13 */
    expect_wire_refusal("wire-version", frame);

    build_identify(frame);
    frame[ADSK_OFF_OPCODE] = ADSK_OP_CREATE; /* opcode 1 is not Identify */
    expect_wire_refusal("wire-opcode", frame);

    build_identify(frame);
    frame[6] = 1u;                          /* reserved byte must be zero */
    expect_wire_refusal("wire-reserved", frame);
}

static void case_foreign_identity(const identify_t *legit) {
    /* The production validator must refuse a name that does not match the
       capability the client actually holds. A foreign/over-broad identity
       is one object ID off the real one — the exact shape a dishonest
       service would return to redirect a client to somebody else's surface. */
    identify_t foreign = *legit;
    foreign.surface_object_id = legit->surface_object_id + 1u;
    int accepted = identify_validate_surface(&foreign);
    report("foreign-identity", 0, accepted == 0 ? 1 : 0, accepted != 0);

    /* The genuine identity must still validate — otherwise the negative
       above would pass for the wrong reason (a validator that refuses
       everything). */
    report("genuine-identity", 0, identify_validate_surface(legit),
           identify_validate_surface(legit) == 0);
}

static void case_unexpected_transfer(void) {
    /* A well-formed Identify that carries a capability instead of CAP_NONE.
       The Desktop must refuse it and dispose of the landed capability; the
       client must still hold a working surface afterwards. */
    uint8_t frame[ADSK_BYTES];
    uint8_t reply[ADSK_BYTES];
    uint64_t out[3];
    build_identify(frame);
    arena_memcpy(reply, frame, ADSK_BYTES);
    out[0] = 0;
    out[1] = 0;
    out[2] = GFX_CAP_NONE;
    int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, a.endpoint_slot, 0, 0,
                                a.surface_slot,          /* NOT CAP_NONE */
                                (uint64_t)(uintptr_t)out,
                                (uint64_t)(uintptr_t)reply);
    int refused = (rc == 0) && (out[0] != 0) && (out[2] == GFX_CAP_NONE);
    report("unexpected-transfer", 1, (long)(refused ? 1 : 0), refused);

    uint64_t desc[3];
    int still_held = describe_slot(a.surface_slot, desc) == 0 &&
                     desc[0] == ARENA_CAP_KIND_SHARED_REGION;
    report("transfer-surface-retained", 1, (long)(still_held ? 1 : 0), still_held);
}

static void case_layout(uint64_t surf, uint64_t ep) {
    int64_t pages = arena_syscall6(ARENA_SYS_SHARED_PAGES, surf, 0, 0, 0, 0, 0);
    int sized = pages >= (int64_t)GFX_TOTAL_PAGES;
    report("layout-pages-minimum", (long)GFX_TOTAL_PAGES,
           (long)(pages < 0 ? 0 : pages), sized);

    /* The pixel window must fit inside the granted region: offset page plus
       300 pixel pages, using the documented 32-bit layout. */
    int fits = pages > 0 &&
               ((uint64_t)pages * ARENA_PAGE_SIZE) >=
                   (uint64_t)PIXEL_OFFSET + (uint64_t)GFX_PIXEL_BYTES &&
               GFX_PIXEL_BYTES == (uint64_t)GFX_W * GFX_H * 4u &&
               PIXEL_OFFSET == ARENA_PAGE_SIZE;
    report("layout-pixel-window-fits", 1, (long)(fits ? 1 : 0), fits);

    /* Real kernel refusal: the same syscall on a capability that is not a
       SharedRegion must fail closed, in the guest, not in a mock. */
    int64_t bad = arena_syscall6(ARENA_SYS_SHARED_PAGES, ep, 0, 0, 0, 0, 0);
    report("layout-wrong-kind-refused", -1, (long)(bad < 0 ? -1 : 0), bad < 0);
}

/* ----------------------------------------------------------------- main --- */

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;

    arena_memset(&a, 0, sizeof(a));
    (void)arena_stdio_init();

    const arena_startup_t *s = arena_startup();
    if (s == NULL || s->present == 0) {
        log_line("[refuse] no startup record");
        arena_exit(GFX_NO_STARTUP);
    }

    uint64_t ep = 0, surf = 0;
    int r = discover(s, &ep, &surf);
    if (r != GFX_OK) {
        log_line("[refuse] startup record was not accepted");
        arena_exit((enum gfx_exit)r);
    }
    a.endpoint_slot = ep;
    a.surface_slot = surf;
    report("baseline-record-accepted", 0, r, r == 0);

    /* Cases 11-14: discovery over the verified record and mutated copies. */
    case_reordering(s, ep, surf);
    case_discovery_refusals(s);

    /* Cases 13-15: the Desktop's production dispatch refusing wire input. */
    case_wire_refusals();

    /* A genuine Identify so the identity cases have a real name to test,
       and so the session is proven still healthy after the refusals. */
    identify_t ident;
    uint8_t id_frame[ADSK_BYTES];
    arena_memset(&ident, 0, sizeof(ident));
    if (identify_exchange(id_frame, &ident) != 0) {
        log_line("[refuse] genuine Identify failed after refusals");
        arena_exit(GFX_IDENTIFY_FAILED);
    }
    report("identify-after-refusals", 0, 0, 1);

    /* Case 15: stale/foreign identity refused by the production validator. */
    case_foreign_identity(&ident);

    /* Case 16: unexpected capability transfer refused and disposed. */
    case_unexpected_transfer();

    /* Case 17: surface-layout boundaries, including a kernel refusal. */
    case_layout(surf, ep);

    log_line(failures == 0 ? "[refuse] matrix complete: all cases refused as required"
                           : "[refuse] matrix complete: FAILURES PRESENT");
    arena_exit(failures == 0 ? GFX_OK : 40 /* distinct: matrix failed */);
    return 0;
}
