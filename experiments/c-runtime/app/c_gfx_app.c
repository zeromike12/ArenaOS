/*
 * c_gfx_app.c — V1 native graphical client (discover → identify → validate → proceed).
 *
 * Integer-only guest application. It locates its Desktop endpoint and its
 * owned surface by kind and rights over the verified ARST v2 record
 * (never by slot), performs the ADR-0114 Identify round-trip (ADSK version
 * 2, opcode 13) over the endpoint, validates the reply against the surface
 * it already holds, maps the shared surface, renders a deterministic
 * integer pattern into it, acknowledges injected keyboard/mouse input
 * visibly, animates for a bounded number of frames, then exits cleanly.
 *
 * Identification is mandatory: the client never renders into or publishes
 * the candidate surface until Identify succeeds and the reply validates
 * against a held capability. On Identify failure, the client fails closed
 * with a distinct exit code — it never falls back to discovery-only
 * operation.
 *
 * Built with experiments/c-runtime under GCC and Clang
 * (run.py build --cc gcc --cc clang) and audited with run.py audit
 * (zero x87/MMX/SSE/AVX instructions, ET_EXEC, no W+X).
 */

#include "arena/abi.h"
#include "arena/rt.h"
#include "arena/string.h"
#include "sysabi.h"

/* In host test builds (ARENA_HOSTED_TEST), TEST_EXPORT is empty so the
   static functions below can be called directly by test_runtime.c. In
   guest builds it is plain static, preserving the production contract. */

#ifdef ARENA_HOSTED_TEST
#define TEST_EXPORT
#else
#define TEST_EXPORT static
#endif

/* ---------------------------------------------------------------- wire ---

   ADSK wire layout (userspace/desktop/src/wire.rs), preserved byte-for-byte
   for ADSK-v1. One 64-byte frame doubles as request and reply buffer:
   b[0..3] = "ADSK", b[4] = version (1), b[5] = opcode, b[6..7] reserved,
   b[8..15] = handle u64 LE, opcode-specific fields at b[16..63]. */

/* ADSK wire versions and opcodes. ADSK-v1 (version byte 1) is preserved
   verbatim for window operations. ADSK-v2 (version byte 2) is used for
   Identify (opcode 13) per ADR-0114. */
#define ADSK_V1 1u
#define ADSK_V2 2u
#define ADSK_OP_IDENTIFY 13u
#define ADSK_OP_CREATE 1u
#define ADSK_OP_DAMAGE 2u
#define ADSK_OP_POLL 3u
#define ADSK_OP_DESTROY_WINDOW 11u

#define ADSK_BYTES 64u
#define ADSK_OFF_VERSION 4u
#define ADSK_OFF_OPCODE 5u
#define ADSK_OFF_HANDLE 8u

/* Identify (ADSK v2) frame layout:
   b[0..3] = "ADSK", b[4] = 2, b[5] = 13, b[6..8] = reserved zero,
   b[8..10] = client protocol version (u16 LE = 1), b[10..64] = reserved zero.
   Reply: b[8..16] = session serial (u64 LE), b[16] = surface kind (7),
   b[17] = surface rights, b[18..26] = surface object ID (u64 LE),
   b[26..64] = reserved zero. */
#define IDENTIFY_CLIENT_PROTOCOL_VERSION 1u
#define IDENTIFY_SURFACE_KIND 7u            /* SharedRegion */
#define IDENTIFY_SURFACE_RIGHTS (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE) /* reply field */
#define IDENTIFY_EXIT_FAILED 14u            /* Identify round-trip failed */

#define ADSK_EVENT_FOCUS 4u
#define ADSK_EVENT_KEY 1u
#define ADSK_EVENT_POINTER 2u
#define ADSK_EVENT_CLOSE 3u

/* ------------------------------------------------------------- surface ---

   V1 surface geometry convention: 640x480 with u32 pixels = 1,228,800
   bytes = exactly 300 pages, placed at pixel offset 4096 (one page) inside
   the granted SharedRegion, so the region must provide at least 301 pages.
   Geometry is a documented convention, never an identity mechanism. */

#define GFX_W 640u
#define GFX_H 480u
#define GFX_PIXEL_BYTES (GFX_W * GFX_H * 4u)
#define GFX_PIXEL_PAGES ((GFX_PIXEL_BYTES + ARENA_PAGE_SIZE - 1u) / ARENA_PAGE_SIZE)
#define GFX_OFFSET_PAGES 1u              /* PIXEL_OFFSET page */
#define GFX_TOTAL_PAGES (GFX_OFFSET_PAGES + GFX_PIXEL_PAGES) /* 301 */
#define PIXEL_OFFSET 4096u
#define GFX_FRAMES 300u
#define GFX_FRAME_US 33333u              /* ~30 fps, bounded busy sleep */
#define GFX_FLASH_FRAMES 30u             /* visible-acknowledgment glow duration */

/* ------------------------------------------------------------------ exit ---

   Distinct positive exit codes: the Desktop records each one through its
   held Process cap as "[desktop] child Process-cap exit status=N", so
   every failure is unambiguous evidence. */

enum gfx_exit {
    GFX_OK = 0,
    GFX_NO_STARTUP = 1,
    GFX_NO_ENDPOINT = 2,
    GFX_AMBIGUOUS_ENDPOINT = 3,
    GFX_NO_SURFACE = 4,
    GFX_AMBIGUOUS_SURFACE = 5,
    GFX_WRONG_RIGHTS = 6,
    GFX_SURFACE_TOO_SMALL = 7,
    GFX_MAP_FAILED = 8,
    GFX_CREATE_FAILED = 9,
    GFX_DAMAGE_FAILED = 10,
    GFX_POLL_FAILED = 11,
    GFX_DESTROY_FAILED = 12,
    GFX_UNMAP_FAILED = 13,
    GFX_IDENTIFY_FAILED = 14,
};

/* Capability kinds/rights as granted by the ADR-0090 desktop profile.
   Role OTHER descriptors are unconstrained in the record, so any number
   of them may be present; kind+rights counting is an existence check
   only, and uniqueness is required. */
#define GFX_ENDPOINT_RIGHTS (ARENA_RIGHT_WRITE | ARENA_RIGHT_COPY | ARENA_RIGHT_DESTROY)
#define GFX_SURFACE_RIGHTS (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE | ARENA_RIGHT_COPY)

#define GFX_CAP_NONE ((uint64_t)0xFFFFFFFFFFFFFFFFull)

#define IDENTIFY_BUSY_RETRIES 512u   /* mirrors app_client.rs IPC_BUSY_RETRIES */

/* ----------------------------------------------------------- app state --- */

typedef struct {
    uint64_t endpoint_slot;
    uint64_t surface_slot;
    uint64_t handle;
    uint8_t *framebuffer;
    int mapped;
} app_t;

#ifdef ARENA_HOSTED_TEST
app_t a;
#else
static app_t a;
#endif

/* ------------------------------------------------------- syscall helpers ---

   arena_syscall6 passes six arguments; the kernel ignores unused trailing
   registers, so two-argument calls pass zeros for the rest. */

TEST_EXPORT int describe_slot(uint64_t slot, uint64_t out[3]) {
    int64_t rc = arena_syscall6(ARENA_SYS_CAP_DESCRIBE, slot,
                                (uint64_t)(uintptr_t)out, 0, 0, 0, 0);
    return rc < 0 ? -1 : 0;
}

/* ---------------------------------------------------------- discovery ---

   Locate the Desktop endpoint and the owned surface by kind + rights over
   the verified record. Slots are recorded only as IPC operands. */

TEST_EXPORT int discover(const arena_startup_t *s, uint64_t *ep, uint64_t *surf) {
    uint64_t ep_exact = 0, ep_any = 0, ep_slot = 0;
    uint64_t surf_exact = 0, surf_any = 0, surf_slot = 0;

    for (unsigned i = 0; i < s->cap_count; i++) {
        const arena_cap_desc_t *c = &s->caps[i];
        if (c->role != ARENA_ARST_ROLE_OTHER) {
            continue;
        }
        if (c->kind == ARENA_CAP_KIND_BADGED_ENDPOINT) {
            ep_any++;
            if (c->rights == GFX_ENDPOINT_RIGHTS) {
                ep_exact++;
                ep_slot = c->slot;
            }
        } else if (c->kind == ARENA_CAP_KIND_SHARED_REGION) {
            surf_any++;
            /* Rights must include READ|WRITE (and COPY, needed to land the
               surface cap on calls): not an exact match on other bits. */
            if ((c->rights & GFX_SURFACE_RIGHTS) == GFX_SURFACE_RIGHTS) {
                surf_exact++;
                surf_slot = c->slot;
            }
        }
    }

    if (ep_any > 0 && ep_exact == 0) return (int)GFX_WRONG_RIGHTS;
    if (ep_exact == 0) return (int)GFX_NO_ENDPOINT;
    if (ep_exact > 1) return (int)GFX_AMBIGUOUS_ENDPOINT;
    if (surf_any > 0 && surf_exact == 0) return (int)GFX_WRONG_RIGHTS;
    if (surf_exact == 0) return (int)GFX_NO_SURFACE;
    if (surf_exact > 1) return (int)GFX_AMBIGUOUS_SURFACE;

    *ep = ep_slot;
    *surf = surf_slot;
    return (int)GFX_OK;
}

/* ------------------------------------------------------------ exchange ---

   One IPC call over the endpoint, landing this session's surface cap so
   the Desktop can authenticate the call (kind 7, this session's object id,
   READ|WRITE|COPY). Retries only kernel STATUS_BUSY (a queue refusal that
   was never accepted); a server status is never retried. An unexpected
   landed capability is destroyed and the call refused. */

TEST_EXPORT int exchange(uint8_t opcode, uint64_t handle, uint16_t w, uint16_t h,
                    uint8_t frame[ADSK_BYTES], uint64_t *result_out) {
    uint64_t out[3];

    arena_memset(frame, 0, ADSK_BYTES);
    arena_memcpy(frame, "ADSK", 4);
    frame[ADSK_OFF_VERSION] = 1u;           /* ADSK-v1 for window ops */
    frame[ADSK_OFF_OPCODE] = opcode;
    for (uint32_t i = 0; i < 8u; i++) {
        frame[ADSK_OFF_HANDLE + i] = (uint8_t)((handle >> (8 * i)) & 0xffu);
    }
    if (opcode == ADSK_OP_CREATE) {
        frame[24] = (uint8_t)(w & 0xffu);
        frame[25] = (uint8_t)((w >> 8) & 0xffu);
        frame[26] = (uint8_t)(h & 0xffu);
        frame[27] = (uint8_t)((h >> 8) & 0xffu);
    } else if (opcode == ADSK_OP_DAMAGE) {
        frame[16] = 0u;                     /* FULL: whole surface */
    }

    out[0] = 0; out[1] = 0; out[2] = GFX_CAP_NONE;
    for (uint32_t attempt = 0; attempt <= IDENTIFY_BUSY_RETRIES; attempt++) {
        int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, a.endpoint_slot, 0, 0,
                                    a.surface_slot,
                                    (uint64_t)(uintptr_t)out,
                                    (uint64_t)(uintptr_t)frame);

        /* An unexpected landed capability is never accepted. */
        if (out[2] != GFX_CAP_NONE) {
            (void)arena_syscall6(ARENA_SYS_CAP_DESTROY, out[2], 0, 0, 0, 0, 0);
            return -1;
        }
        if (rc == ARENA_STATUS_BUSY && attempt < IDENTIFY_BUSY_RETRIES) {
            uint64_t delay = (1000ull << (attempt > 5u ? 5u : attempt));
            arena_busy_sleep_us(delay);
            continue;
        }
        if (rc != 0 || out[0] != 0) {
            return -1;
        }
        if (result_out != NULL) {
            *result_out = out[1];
        }
        return 0;
    }
    return -1;
}

/* ------------------------------------------------------------ identify ---

   Perform the ADR-0114 Identify round-trip (ADSK version 2, opcode 13)
   over the endpoint. Sends CAP_NONE (no capability transfer). On success,
   fills `ident` with the Desktop's named surface identity. On any failure
   (kernel error, service error, malformed reply, wrong kind/rights/object
   ID mismatch against the held surface) returns -1 and the caller must
   fail closed — never fall back to discovery-only operation. */

typedef struct {
    uint64_t session_serial;
    uint64_t surface_object_id;
} identify_t;

TEST_EXPORT int identify_exchange(uint8_t frame[ADSK_BYTES], identify_t *ident) {
    uint64_t out[3];

    /* Build the Identify request: ADSK v2, opcode 13, client protocol
       version 1, all other bytes zero. */
    arena_memset(frame, 0, ADSK_BYTES);
    arena_memcpy(frame, "ADSK", 4);
    frame[ADSK_OFF_VERSION] = ADSK_V2;
    frame[ADSK_OFF_OPCODE] = ADSK_OP_IDENTIFY;
    frame[8] = (uint8_t)(IDENTIFY_CLIENT_PROTOCOL_VERSION & 0xffu);
    frame[9] = (uint8_t)((IDENTIFY_CLIENT_PROTOCOL_VERSION >> 8) & 0xffu);

    out[0] = 0; out[1] = 0; out[2] = GFX_CAP_NONE;
    for (uint32_t attempt = 0; attempt <= IDENTIFY_BUSY_RETRIES; attempt++) {
        int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, a.endpoint_slot, 0, 0,
                                    a.surface_slot,
                                    (uint64_t)(uintptr_t)out,
                                    (uint64_t)(uintptr_t)frame);

        /* An unexpected landed capability is never accepted. */
        if (out[2] != GFX_CAP_NONE) {
            (void)arena_syscall6(ARENA_SYS_CAP_DESTROY, out[2], 0, 0, 0, 0, 0);
            return -1;
        }
        /* Kernel result must be success before trusting the payload. */
        if (rc != 0) {
            return -1;
        }
        /* Service status word out[0] carries the typed service error. */
        if (out[0] != 0) {
            return -1;
        }
        /* Validate the reply frame: magic, version 2, opcode 13, reserved
           bytes zero, and echo of our request bytes where applicable. */
        if (arena_memcmp(frame, "ADSK", 4) != 0 ||
            frame[ADSK_OFF_VERSION] != ADSK_V2 ||
            frame[ADSK_OFF_OPCODE] != ADSK_OP_IDENTIFY ||
            frame[6] != 0 || frame[7] != 0) {
            return -1;
        }
        for (uint32_t i = 26u; i < ADSK_BYTES; i++) {
            if (frame[i] != 0) return -1;
        }

        ident->session_serial = 0;
        for (uint32_t i = 0; i < 8u; i++) {
            ident->session_serial |=
                (uint64_t)frame[8 + i] << (8 * i);
        }
        ident->surface_object_id = 0;
        for (uint32_t i = 0; i < 8u; i++) {
            ident->surface_object_id |=
                (uint64_t)frame[18 + i] << (8 * i);
        }
        uint8_t surface_kind = frame[16];
        uint8_t surface_rights = frame[17];

        /* The reply must name a SharedRegion with READ|WRITE rights. */
        if (surface_kind != IDENTIFY_SURFACE_KIND ||
            (surface_rights & (uint8_t)(ARENA_RIGHT_READ | ARENA_RIGHT_WRITE)) !=
                (uint8_t)(ARENA_RIGHT_READ | ARENA_RIGHT_WRITE)) {
            return -1;
        }

        /* Validate the named surface against a capability the client
           already holds: SYS_CAP_DESCRIBE(surface_slot) must return
           [kind 7, object_id == ident->surface_object_id, rights including
           READ|WRITE|COPY]. */
        uint64_t desc[3];
        if (describe_slot(a.surface_slot, desc) != 0 ||
            desc[0] != ARENA_CAP_KIND_SHARED_REGION ||
            desc[1] != ident->surface_object_id ||
            (desc[2] & GFX_SURFACE_RIGHTS) != GFX_SURFACE_RIGHTS) {
            return -1;
        }

        return 0;
    }
    return -1;
}

TEST_EXPORT uint64_t create_window(uint16_t w, uint16_t h) {
    uint64_t handle = 0;
    uint8_t frame[ADSK_BYTES];
    if (exchange(ADSK_OP_CREATE, 0, w, h, frame, &handle) != 0 || handle == 0) {
        return 0;
    }
    return handle;
}

TEST_EXPORT int damage(uint64_t handle) {
    uint64_t result = 0;
    uint8_t frame[ADSK_BYTES];
    return exchange(ADSK_OP_DAMAGE, handle, 0, 0, frame, &result) == 0;
}

/* Returns 1 when an event frame was decoded, 0 when none, -1 on failure. */

TEST_EXPORT int poll_event(uint8_t reply[ADSK_BYTES]) {
    uint64_t result = 0;
    if (exchange(ADSK_OP_POLL, a.handle, 0, 0, reply, &result) != 0) {
        return -1;
    }
    /* The reply buffer holds the server's inline reply. An event frame is
       ADSK-v1 opcode 5; anything else (including the echoed Poll frame,
       opcode 3, when nothing is queued) means no event this turn. */
    if (arena_memcmp(reply, "ADSK", 4) != 0 ||
        reply[ADSK_OFF_VERSION] != 1u || reply[ADSK_OFF_OPCODE] != 5u) {
        return 0;
    }
    for (uint32_t i = 6u; i < 8u; i++) {
        if (reply[i] != 0) return 0;
    }
    for (uint32_t i = 30u; i < ADSK_BYTES; i++) {
        if (reply[i] != 0) return 0;
    }
    /* Echoed handle must match this window. */
    uint64_t echo = 0;
    for (uint32_t i = 0; i < 8u; i++) {
        echo |= (uint64_t)reply[ADSK_OFF_HANDLE + i] << (8 * i);
    }
    if (echo != a.handle) {
        return 0;
    }
    return 1; /* event type at reply[28], detail at reply[29] */
}

/* ------------------------------------------------------------- render ---- */

TEST_EXPORT void render_frame(uint32_t *pixels, uint32_t frame, int flash) {
    for (uint32_t y = 0; y < GFX_H; y++) {
        for (uint32_t x = 0; x < GFX_W; x++) {
            uint32_t cell = ((x >> 4) + (y >> 4) + frame) & 1u;
            uint32_t r = cell ? 60u : 20u;
            uint32_t g = 40u;
            uint32_t b = cell ? 20u : 90u;
            uint32_t px = (r << 16) | (g << 8) | b; /* 0x00RRGGBB */
            if (flash) {
                px ^= 0x00A0A0A0u;
            }
            pixels[y * GFX_W + x] = px;
        }
    }
}

/* --------------------------------------------------------------- logging ---

   Best effort over the granted stdout stream when one was granted; silent
   otherwise. No serial/debug fallback ever. */

TEST_EXPORT void log_line(const char *msg) {
    char buf[96];
    size_t n = arena_strnlen(msg, sizeof(buf) - 1u);
    arena_memcpy(buf, msg, n);
    buf[n] = '\n';
    (void)arena_write(1, buf, n + 1u);
}

/* ---------------------------------------------------------------- main --- */

#ifndef ARENA_HOSTED_TEST
int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    arena_memset(&a, 0, sizeof(a));
    (void)arena_stdio_init();

    const arena_startup_t *s = arena_startup();
    if (s == NULL || s->present == 0) {
        log_line("[gfx] no startup record");
        arena_exit(GFX_NO_STARTUP);
    }

    uint64_t ep = 0, surf = 0;
    int r = discover(s, &ep, &surf);
    if (r != GFX_OK) {
        static const char *const msgs[] = {
            "[gfx] discovery ok",      /* 0: unused */
            "[gfx] discovery ok",      /* 1: unused */
            "[gfx] no endpoint",       /* GFX_NO_ENDPOINT = 2 */
            "[gfx] ambiguous endpoint",/* 3 */
            "[gfx] no surface",        /* GFX_NO_SURFACE = 4 */
            "[gfx] ambiguous surface", /* 5 */
            "[gfx] wrong rights",      /* GFX_WRONG_RIGHTS = 6 */
        };
        const char *msg = (r >= 2 && r <= 6) ? msgs[r] : "[gfx] discovery failed";
        log_line(msg);
        arena_exit((enum gfx_exit)r);
    }
    a.endpoint_slot = ep;
    a.surface_slot = surf;

    uint64_t desc[3];
    if (describe_slot(surf, desc) != 0 || desc[0] != ARENA_CAP_KIND_SHARED_REGION ||
        (desc[2] & GFX_SURFACE_RIGHTS) != GFX_SURFACE_RIGHTS) {
        log_line("[gfx] surface describe mismatch");
        arena_exit(GFX_WRONG_RIGHTS);
    }

    int64_t pages = arena_syscall6(ARENA_SYS_SHARED_PAGES, surf, 0, 0, 0, 0, 0);
    if (pages < 0 || (uint64_t)pages < GFX_TOTAL_PAGES) {
        log_line("[gfx] surface too small");
        arena_exit(GFX_SURFACE_TOO_SMALL);
    }

    int64_t va = arena_syscall6(ARENA_SYS_SHARED_MAP, surf, 1u, 0, 0, 0, 0);
    if (va <= 0) {
        log_line("[gfx] map failed");
        arena_exit(GFX_MAP_FAILED);
    }
    a.framebuffer = (uint8_t *)(uintptr_t)va;
    a.mapped = 1;
    uint32_t *pixels = (uint32_t *)(a.framebuffer + PIXEL_OFFSET);

    /* Perform the ADR-0114 Identify round-trip before any graphical
       request. Identify is mandatory — failure means fail closed, never
       fall back to discovery-only operation. */

    uint8_t id_frame[ADSK_BYTES];
    identify_t ident;
    if (identify_exchange(id_frame, &ident) != 0) {
        log_line("[gfx] identify failed");
        if (a.mapped) {
            (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                 (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
        }
        arena_exit(GFX_IDENTIFY_FAILED);
    }
    (void)ident; /* session_serial and surface_object_id validated against
                    the held surface inside identify_exchange */

    uint64_t handle = create_window(GFX_W, GFX_H);
    if (handle == 0) {
        log_line("[gfx] create failed");
        if (a.mapped) {
            (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                 (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
        }
        arena_exit(GFX_CREATE_FAILED);
    }
    a.handle = handle;
    log_line("[gfx] window created");

    uint32_t flash = 0;
    uint64_t events = 0;
    int focused = -1;
    char line[96];
    int exit_code = GFX_OK;

    for (uint32_t f = 0; f < GFX_FRAMES; f++) {
        render_frame(pixels, f, flash > 0 ? 1 : 0);
        if (flash > 0) flash--;

        if (!damage(handle)) {
            log_line("[gfx] damage failed");
            exit_code = GFX_DAMAGE_FAILED;
            break;
        }

        uint8_t reply[ADSK_BYTES];
        int pr = poll_event(reply);
        if (pr < 0) {
            log_line("[gfx] poll failed");
            exit_code = GFX_POLL_FAILED;
            break;
        }
        if (pr == 1) {
            uint8_t etype = reply[28];
            if (etype == ADSK_EVENT_FOCUS) {
                focused = reply[29] ? 1 : 0;
                (void)arena_snprintf(line, sizeof(line),
                                     "[gfx] focus=%d", focused);
                log_line(line);
            } else if (etype == ADSK_EVENT_KEY || etype == ADSK_EVENT_POINTER) {
                events++;
                flash = GFX_FLASH_FRAMES;
                (void)arena_snprintf(line, sizeof(line),
                                     "[gfx] event type=%u events=%llu",
                                     (unsigned)etype,
                                     (unsigned long long)events);
                log_line(line);
            } else if (etype == ADSK_EVENT_CLOSE) {
                break;
            }
        }

        arena_busy_sleep_us(GFX_FRAME_US);
    }

    /* Teardown: DestroyWindow over the endpoint, then unmap the surface.
       Policy:
         - Preserve an existing primary failure (exit_code).
         - Attempt necessary cleanup regardless.
         - If the operation was otherwise successful but required cleanup
           fails, return a distinct failure (GFX_DESTROY_FAILED / GFX_UNMAP_FAILED).
         - Record cleanup failures even when preserving an earlier primary
           failure.
         - Do not report clean teardown merely because process exit may
           eventually reclaim resources. */
    {
        int destroy_failed = 0;
        int unmap_failed = 0;
        uint8_t frame[ADSK_BYTES];
        uint64_t result = 0;
        if (exchange(ADSK_OP_DESTROY_WINDOW, handle, 0, 0, frame, &result) != 0) {
            log_line("[gfx] destroy failed");
            destroy_failed = 1;
        }
        if (a.mapped) {
            int64_t urc = arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                         (uint64_t)(uintptr_t)a.framebuffer,
                                         0, 0, 0, 0, 0);
            if (urc != 0) {
                log_line("[gfx] unmap failed");
                unmap_failed = 1;
            }
        }

        if (exit_code == GFX_OK) {
            if (destroy_failed) {
                exit_code = GFX_DESTROY_FAILED;
            } else if (unmap_failed) {
                exit_code = GFX_UNMAP_FAILED;
            }
        }
        /* exit_code != GFX_OK: primary failure is preserved; cleanup
           failures above are already logged and not overwritten. */
    }

    (void)arena_snprintf(line, sizeof(line),
                         "[gfx] exit events=%llu focused=%d code=%d",
                         (unsigned long long)events, focused, exit_code);
    log_line(line);
    arena_exit(exit_code);
    return 0;
}
#endif /* ARENA_HOSTED_TEST */
