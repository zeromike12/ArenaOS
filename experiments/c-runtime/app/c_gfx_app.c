/*
 * c_gfx_app.c — V1 native graphical client scaffolding (ADR-0114).
 *
 * Integer-only guest application. It locates its Desktop endpoint and its
 * owned surface by kind and rights over the verified ARST v2 record
 * (never by slot), performs the versioned Identify round-trip over the
 * endpoint, renders a deterministic integer pattern into the shared
 * surface, acknowledges injected keyboard/mouse input visibly, animates
 * for a bounded number of frames, then exits cleanly.
 *
 * The Identify round-trip is mandatory: without an implementing Desktop the
 * app fails closed at identification and never guesses which endpoint or
 * surface is its own.
 *
 * Built with experiments/c-runtime under GCC and Clang
 * (run.py build --cc gcc --cc clang) and audited with run.py audit
 * (zero x87/MMX/SSE/AVX instructions, ET_EXEC, no W+X).
 */

#include "arena/abi.h"
#include "arena/rt.h"
#include "arena/string.h"
#include "sysabi.h"

/* ---------------------------------------------------------------- wire ---

   ADSK wire layout (userspace/desktop/src/wire.rs), preserved byte-for-byte
   for ADSK-v1. One 64-byte frame doubles as request and reply buffer:
   b[0..3] = "ADSK", b[4] = version, b[5] = opcode, b[6..7] reserved,
   b[8..15] = handle u64 LE, opcode-specific fields at b[16..63]. */

/* ADSK-v1 opcodes the client emits. */
#define ADSK_OP_CREATE 1u
#define ADSK_OP_DAMAGE 2u
#define ADSK_OP_POLL 3u
#define ADSK_OP_DESTROY_WINDOW 11u

/* ADR-0114: Identify runs over ADSK version 2, opcode 13. */
#define ADSK_VERSION_IDENTIFY 2u
#define ADSK_OP_IDENTIFY 13u
#define IDENTIFY_CLIENT_VERSION 1u

#define ADSK_BYTES 64u
#define ADSK_OFF_VERSION 4u
#define ADSK_OFF_OPCODE 5u
#define ADSK_OFF_HANDLE 8u
#define ADSK_OFF_REPLY_STATUS 6u
#define ADSK_OFF_REPLY_SERIAL 8u
#define ADSK_OFF_REPLY_KIND 16u
#define ADSK_OFF_REPLY_RIGHTS 17u
#define ADSK_OFF_REPLY_GENERATION 18u

#define ADSK_EVENT_FOCUS 4u
#define ADSK_EVENT_KEY 1u
#define ADSK_EVENT_POINTER 2u
#define ADSK_EVENT_CLOSE 3u

/* ------------------------------------------------------------- surface ---

   V1 surface geometry convention: 640x480 RGB888 = 921,600 bytes =
   exactly 225 pages, inside the wire's 80x60..1024x768 bounds. Geometry is
   a documented convention, never an identity mechanism. */

#define GFX_W 640u
#define GFX_H 480u
#define GFX_BPP 3u
#define GFX_BYTES (GFX_W * GFX_H * GFX_BPP)
#define GFX_PAGES ((GFX_BYTES + ARENA_PAGE_SIZE - 1u) / ARENA_PAGE_SIZE)
#define GFX_FRAMES 300u
#define GFX_FRAME_US 33333u     /* ~30 fps, bounded busy sleep */
#define GFX_FLASH_FRAMES 30u    /* visible-acknowledgment glow duration */

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
    GFX_IDENTIFY_FAILED = 9,
    GFX_CREATE_FAILED = 10,
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
    uint64_t surface_generation;
    uint64_t own_badge;
    uint8_t *framebuffer;
    int mapped;
} app_t;

/* ------------------------------------------------------- syscall helpers ---

   arena_syscall6 passes six arguments; the kernel ignores unused trailing
   registers, so two-argument calls pass zeros for the rest. */

static int describe_slot(uint64_t slot, uint64_t out[3]) {
    int64_t rc = arena_syscall6(ARENA_SYS_CAP_DESCRIBE, slot,
                                (uint64_t)(uintptr_t)out, 0, 0, 0, 0);
    return rc < 0 ? -1 : 0;
}

/* ---------------------------------------------------------- discovery ---

   Locate the Desktop endpoint and the owned surface by kind + rights over
   the verified record. Slots are recorded only as IPC operands. */

static int discover(const arena_startup_t *s, uint64_t *ep, uint64_t *surf) {
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
            if (c->rights == GFX_SURFACE_RIGHTS) {
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

/* ----------------------------------------------------------- identify ---

   One bounded Identify round-trip over the endpoint (ADSK v2, opcode 13).
   The reply manufactures no authority: it is accepted only when its named
   surface matches a capability this process already holds. `s` is the
   verified startup record; `surface_slot` is the unique exact-rights
   candidate discovered above with generation `surface_generation`. */

static int identify(const arena_startup_t *s, uint64_t ep,
                    uint64_t surface_slot, uint64_t surface_generation,
                    uint64_t own_badge) {
    uint8_t frame[ADSK_BYTES];
    uint64_t out[3];

    for (uint32_t attempt = 0; attempt <= IDENTIFY_BUSY_RETRIES; attempt++) {
        arena_memset(frame, 0, ADSK_BYTES);
        arena_memcpy(frame, "ADSK", 4);
        frame[ADSK_OFF_VERSION] = (uint8_t)ADSK_VERSION_IDENTIFY;
        frame[ADSK_OFF_OPCODE] = (uint8_t)ADSK_OP_IDENTIFY;
        frame[8] = (uint8_t)(IDENTIFY_CLIENT_VERSION & 0xffu);
        frame[9] = (uint8_t)((IDENTIFY_CLIENT_VERSION >> 8) & 0xffu);

        out[0] = 0; out[1] = 0; out[2] = GFX_CAP_NONE;
        int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, ep, 0, 0,
                                    GFX_CAP_NONE,
                                    (uint64_t)(uintptr_t)out,
                                    (uint64_t)(uintptr_t)frame);

        if (rc == ARENA_STATUS_BUSY) {
            if (attempt < IDENTIFY_BUSY_RETRIES) {
                uint64_t delay = (1000ull << (attempt > 5u ? 5u : attempt));
                arena_busy_sleep_us(delay);
                continue;
            }
            return (int)GFX_IDENTIFY_FAILED;
        }
        if (rc != 0) {
            return (int)GFX_IDENTIFY_FAILED;
        }
        /* A server status was returned: this request was processed. Never
           retry a server status, and an unexpected returned cap is
           disposed of as a refusal. */
        if (out[0] != 0 || out[2] != GFX_CAP_NONE) {
            if (out[2] != GFX_CAP_NONE) {
                (void)arena_syscall6(ARENA_SYS_CAP_DESTROY, out[2], 0, 0, 0, 0, 0);
            }
            return (int)GFX_IDENTIFY_FAILED;
        }

        /* Strict reply shape. */
        if (arena_memcmp(frame, "ADSK", 4) != 0 ||
            frame[ADSK_OFF_VERSION] != (uint8_t)ADSK_VERSION_IDENTIFY ||
            frame[ADSK_OFF_OPCODE] != (uint8_t)ADSK_OP_IDENTIFY ||
            frame[ADSK_OFF_REPLY_STATUS] != 0 || frame[7] != 0) {
            return (int)GFX_IDENTIFY_FAILED;
        }
        for (uint32_t i = 26u; i < ADSK_BYTES; i++) {
            if (frame[i] != 0) return (int)GFX_IDENTIFY_FAILED;
        }

        /* Echo check: the reply's session serial must be this endpoint's
           own kernel-delivered badge (a spliced reply names another
           session). Bytes 19..25 are the high part of the generation and
           are excluded from this reserved check above only as generation. */
        uint64_t serial = 0;
        uint64_t reply_gen = 0;
        for (uint32_t i = 0; i < 8u; i++) {
            serial |= (uint64_t)frame[ADSK_OFF_REPLY_SERIAL + i] << (8 * i);
            reply_gen |= (uint64_t)frame[ADSK_OFF_REPLY_GENERATION + i] << (8 * i);
        }
        if (serial != own_badge) {
            return (int)GFX_IDENTIFY_FAILED;
        }
        if (frame[ADSK_OFF_REPLY_KIND] != (uint8_t)ARENA_CAP_KIND_SHARED_REGION ||
            frame[ADSK_OFF_REPLY_RIGHTS] != (uint8_t)(GFX_SURFACE_RIGHTS & 0xffu) ||
            reply_gen != surface_generation) {
            return (int)GFX_IDENTIFY_FAILED;
        }

        /* Cross-check: exactly one of this process's held SharedRegions
           carries kind 7, rights READ|WRITE|COPY, and this generation,
           and it is the surface we discovered. */
        unsigned matches = 0;
        uint64_t match_slot = 0;
        for (unsigned i = 0; i < s->cap_count; i++) {
            const arena_cap_desc_t *c = &s->caps[i];
            if (c->kind != ARENA_CAP_KIND_SHARED_REGION ||
                c->rights != GFX_SURFACE_RIGHTS) {
                continue;
            }
            uint64_t d[3];
            if (describe_slot(c->slot, d) != 0 || d[0] != ARENA_CAP_KIND_SHARED_REGION ||
                d[2] != GFX_SURFACE_RIGHTS) {
                continue;
            }
            if (d[1] == surface_generation) {
                matches++;
                match_slot = c->slot;
            }
        }
        if (matches != 1u || match_slot != surface_slot) {
            return (int)GFX_IDENTIFY_FAILED;
        }

        return (int)GFX_OK;
    }
    return (int)GFX_IDENTIFY_FAILED;
}

/* ---------------------------------------------------------------- create --- */

static uint64_t create_window(uint64_t ep) {
    uint8_t frame[ADSK_BYTES];
    uint64_t out[3];

    arena_memset(frame, 0, ADSK_BYTES);
    arena_memcpy(frame, "ADSK", 4);
    frame[ADSK_OFF_VERSION] = 1u;           /* ADSK-v1 for window ops */
    frame[ADSK_OFF_OPCODE] = (uint8_t)ADSK_OP_CREATE;
    frame[24] = (uint8_t)(GFX_W & 0xffu);
    frame[25] = (uint8_t)((GFX_W >> 8) & 0xffu);
    frame[26] = (uint8_t)(GFX_H & 0xffu);
    frame[27] = (uint8_t)((GFX_H >> 8) & 0xffu);

    out[0] = 0; out[1] = 0; out[2] = GFX_CAP_NONE;
    int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, ep, 0, 0, GFX_CAP_NONE,
                                (uint64_t)(uintptr_t)out,
                                (uint64_t)(uintptr_t)frame);
    (void)rc;
    return out[1]; /* handle: 0 on refusal */
}

/* ---------------------------------------------------------------- damage ---

   Publish the whole surface (n == 0 means full damage). */

static int damage(uint64_t ep, uint64_t handle) {
    uint8_t frame[ADSK_BYTES];
    uint64_t out[3];

    arena_memset(frame, 0, ADSK_BYTES);
    arena_memcpy(frame, "ADSK", 4);
    frame[ADSK_OFF_VERSION] = 1u;
    frame[ADSK_OFF_OPCODE] = (uint8_t)ADSK_OP_DAMAGE;
    frame[16] = 0u; /* FULL: whole surface */
    for (uint32_t i = 0; i < 8u; i++) {
        frame[ADSK_OFF_HANDLE + i] = (uint8_t)((handle >> (8 * i)) & 0xffu);
    }

    out[0] = 0; out[1] = 0; out[2] = GFX_CAP_NONE;
    int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, ep, 0, 0, GFX_CAP_NONE,
                                (uint64_t)(uintptr_t)out,
                                (uint64_t)(uintptr_t)frame);
    (void)rc;
    return out[0] == 0 ? 1 : 0;
}

/* ---------------------------------------------------------------- poll ---

   Wait once for an event on the window; the reply buffer is reused as the
   request buffer. Returns 1 when an Event frame was decoded, 0 when none,
   negative on an IPC failure the caller treats as fatal. */

static int poll_event(uint64_t ep, uint64_t handle, uint8_t reply[ADSK_BYTES]) {
    arena_memset(reply, 0, ADSK_BYTES);
    arena_memcpy(reply, "ADSK", 4);
    reply[ADSK_OFF_VERSION] = 1u;
    reply[ADSK_OFF_OPCODE] = (uint8_t)ADSK_OP_POLL;
    for (uint32_t i = 0; i < 8u; i++) {
        reply[ADSK_OFF_HANDLE + i] = (uint8_t)((handle >> (8 * i)) & 0xffu);
    }

    uint64_t out[3];
    out[0] = 0; out[1] = 0; out[2] = GFX_CAP_NONE;
    int64_t rc = arena_syscall6(ARENA_SYS_IPC_CALL, ep, 0, 0, GFX_CAP_NONE,
                                (uint64_t)(uintptr_t)out,
                                (uint64_t)(uintptr_t)reply);
    if (rc != 0 || out[0] != 0) {
        return -1;
    }
    if (arena_memcmp(reply, "ADSK", 4) != 0 || reply[ADSK_OFF_OPCODE] != 5u) {
        return 0; /* not an event frame: no input this turn */
    }
    return 1;
}

/* ------------------------------------------------------------- render ---- */

static void render_frame(uint8_t *fb, uint32_t frame, int flash) {
    for (uint32_t y = 0; y < GFX_H; y++) {
        for (uint32_t x = 0; x < GFX_W; x++) {
            uint32_t cell = ((x >> 4) + (y >> 4) + frame) & 1u;
            uint32_t r = cell ? 60u : 20u;
            uint32_t g = 40u;
            uint32_t b = cell ? 20u : 90u;
            if (flash) {
                r ^= 0xA0u;
                g ^= 0xA0u;
                b ^= 0xA0u;
            }
            size_t off = (size_t)((y * GFX_W + x) * GFX_BPP);
            fb[off] = (uint8_t)b;
            fb[off + 1] = (uint8_t)g;
            fb[off + 2] = (uint8_t)r;
        }
    }
}

/* ----------------------------------------------------------- logging -----

   Best effort over the granted stdout stream when one was granted; silent
   otherwise. No serial/debug fallback ever. */

static void log_line(const char *msg) {
    char buf[96];
    size_t n = arena_strnlen(msg, sizeof(buf) - 1u);
    arena_memcpy(buf, msg, n);
    buf[n] = '\n';
    (void)arena_write(1, buf, n + 1u);
}

/* ---------------------------------------------------------------- main --- */

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
    app_t a;
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
            "[gfx] discovery ok",   /* 0: unused */
            "[gfx] no startup record", /* 1: handled above */
            "[gfx] no endpoint",          /* GFX_NO_ENDPOINT = 2 */
            "[gfx] ambiguous endpoint",   /* 3 */
            "[gfx] wrong rights",         /* 4: endpoint any>0, exact==0 */
            "[gfx] no surface",           /* GFX_NO_SURFACE = 5 */
            "[gfx] ambiguous surface",    /* 6 */
            "[gfx] wrong rights",         /* 7: surface any>0, exact==0 */
        };
        const char *msg = (r >= 2 && r <= 7) ? msgs[r] : "[gfx] discovery failed";
        log_line(msg);
        arena_exit((enum gfx_exit)r);
    }
    a.endpoint_slot = ep;
    a.surface_slot = surf;

    uint64_t desc[3];
    if (describe_slot(surf, desc) != 0 || desc[0] != ARENA_CAP_KIND_SHARED_REGION ||
        desc[2] != GFX_SURFACE_RIGHTS) {
        log_line("[gfx] surface describe mismatch");
        arena_exit(GFX_WRONG_RIGHTS);
    }
    a.surface_generation = desc[1];

    int64_t pages = arena_syscall6(ARENA_SYS_SHARED_PAGES, surf, 0, 0, 0, 0, 0);
    if (pages < 0 || (uint64_t)pages < GFX_PAGES) {
        log_line("[gfx] surface too small");
        arena_exit(GFX_SURFACE_TOO_SMALL);
    }

    int64_t va = arena_syscall6(ARENA_SYS_SHARED_MAP, surf, GFX_PAGES, 0, 0, 0, 0);
    if (va <= 0) {
        log_line("[gfx] map failed");
        arena_exit(GFX_MAP_FAILED);
    }
    a.framebuffer = (uint8_t *)(uintptr_t)va;
    a.mapped = 1;

    int64_t badge = arena_syscall6(ARENA_SYS_ENDPOINT_BADGE, ep, ep, 0, 0, 0, 0);
    if (badge < 0) {
        log_line("[gfx] badge query failed");
        if (a.mapped) {
            (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                 (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
        }
        arena_exit(GFX_IDENTIFY_FAILED);
    }
    a.own_badge = (uint64_t)badge;

    if (identify(s, ep, surf, a.surface_generation, a.own_badge) != GFX_OK) {
        log_line("[gfx] identify failed");
        if (a.mapped) {
            (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                 (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
        }
        arena_exit(GFX_IDENTIFY_FAILED);
    }
    log_line("[gfx] identified endpoint and surface");

    uint64_t handle = create_window(ep);
    if (handle == 0) {
        log_line("[gfx] create failed");
        if (a.mapped) {
            (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                 (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
        }
        arena_exit(GFX_CREATE_FAILED);
    }

    uint32_t flash = 0;
    uint64_t events = 0;
    int focused = -1;
    char line[96];

    for (uint32_t f = 0; f < GFX_FRAMES; f++) {
        render_frame(a.framebuffer, f, flash > 0 ? 1 : 0);
        if (flash > 0) flash--;

        if (!damage(ep, handle)) {
            log_line("[gfx] damage failed");
            if (a.mapped) {
                (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                     (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
            }
            arena_exit(GFX_IDENTIFY_FAILED);
        }

        uint8_t reply[ADSK_BYTES];
        int pr = poll_event(ep, handle, reply);
        if (pr < 0) {
            log_line("[gfx] poll failed");
            if (a.mapped) {
                (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                                     (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
            }
            arena_exit(GFX_IDENTIFY_FAILED);
        }
        if (pr == 1) {
            uint8_t etype = reply[28];
            if (etype == ADSK_EVENT_FOCUS) {
                focused = reply[29] ? 1 : 0;
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

    /* Teardown: DestroyWindow over the endpoint, unmap, exit cleanly. */
    {
        uint8_t frame[ADSK_BYTES];
        uint64_t out[3];
        arena_memset(frame, 0, ADSK_BYTES);
        arena_memcpy(frame, "ADSK", 4);
        frame[ADSK_OFF_VERSION] = 1u;
        frame[ADSK_OFF_OPCODE] = (uint8_t)ADSK_OP_DESTROY_WINDOW;
        for (uint32_t i = 0; i < 8u; i++) {
            frame[ADSK_OFF_HANDLE + i] = (uint8_t)((handle >> (8 * i)) & 0xffu);
        }
        out[0] = 0; out[1] = 0; out[2] = GFX_CAP_NONE;
        (void)arena_syscall6(ARENA_SYS_IPC_CALL, a.endpoint_slot, 0, 0,
                             GFX_CAP_NONE, (uint64_t)(uintptr_t)out,
                             (uint64_t)(uintptr_t)frame);
    }
    if (a.mapped) {
        (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP,
                             (uint64_t)(uintptr_t)a.framebuffer, 0, 0, 0, 0, 0);
    }

    (void)arena_snprintf(line, sizeof(line),
                         "[gfx] exit events=%llu focused=%d",
                         (unsigned long long)events, focused);
    log_line(line);
    arena_exit(GFX_OK);
    return 0;
}
