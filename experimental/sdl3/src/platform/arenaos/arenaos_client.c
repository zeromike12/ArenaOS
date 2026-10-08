#include "arenaos_client.h"
#include "arenaos_syscalls.h"

#define ADSK_BYTES 64
#define IPC_BUSY_RETRIES 512
#define PIXEL_OFFSET 4096

static uint64_t s_handle = 0;
static void *s_surface_va = NULL;
static uint16_t s_width = 0;
static uint16_t s_height = 0;

static void retry_after_busy(size_t attempt) {
    uint64_t now = arenaos_clock_now();
    uint64_t shift = (attempt < 5) ? attempt : 5;
    uint64_t delay = (1000ULL << shift) + (now & 0x7ff);
    int64_t timer_id = arenaos_syscall3(SYS_TIMER_ARM, CLOCK_SLOT, 1, delay);
    if (timer_id >= 0) {
        int64_t w = arenaos_syscall1(SYS_WAIT, CLOCK_SLOT);
        if (w < 0) {
            /* If wait was aborted before badge delivery, disarm the timer */
            arenaos_syscall1(SYS_TIMER_CANCEL, (uint64_t)timer_id);
        }
    }
}

void adsk_sleep_ms(uint32_t ms) {
    if (ms == 0) return;
    uint64_t us = (uint64_t)ms * 1000ULL;
    int64_t timer_id = arenaos_syscall3(SYS_TIMER_ARM, CLOCK_SLOT, 1, us);
    if (timer_id >= 0) {
        int64_t w = arenaos_syscall1(SYS_WAIT, CLOCK_SLOT);
        if (w < 0) {
            /* If wait was aborted before badge delivery, disarm the timer */
            arenaos_syscall1(SYS_TIMER_CANCEL, (uint64_t)timer_id);
        }
    }
}

static int exchange(uint8_t frame_buf[ADSK_BYTES], uint64_t out[3]) {
    arenaos_debug_write("[sdl3-app] -> exchange\n", 23);
    for (size_t attempt = 0; attempt <= IPC_BUSY_RETRIES; attempt++) {
        out[0] = 0;
        out[1] = 0;
        out[2] = CAP_NONE;

        int64_t rc = arenaos_syscall6(
            SYS_IPC_CALL,
            SERVICE_ENDPOINT,
            0,
            0,
            SURFACE_SLOT,
            (uint64_t)out,
            (uint64_t)frame_buf
        );

        if (out[2] != CAP_NONE) {
            arenaos_syscall1(SYS_CAP_DESTROY, out[2]);
            return -2;
        }

        if (rc == STATUS_BUSY && attempt < IPC_BUSY_RETRIES) {
            retry_after_busy(attempt);
            continue;
        }

        if (rc != 0) {
            return (int)rc;
        }

        if (out[0] != 0) {
            return -2;
        }

        /* Check magic */
        if (frame_buf[0] != 'A' || frame_buf[1] != 'D' ||
            frame_buf[2] != 'S' || frame_buf[3] != 'K' ||
            frame_buf[4] != 1) {
            return -2;
        }

        return 0;
    }
    return -4;
}

bool adsk_connect(uint16_t width, uint16_t height, const char *title) {
    arenaos_debug_write("[sdl3-app] -> adsk_connect\n", 27);
    if (width < 80 || width > 800 || height < 60 || height > 600) {
        return false;
    }

    uint64_t bound[2] = {0, 0};
    int64_t rc = arenaos_syscall6(
        SYS_SHARED_INFO,
        SURFACE_SLOT,
        (uint64_t)bound,
        0, 0, 0, 0
    );
    if (rc != 0 || bound[0] == 0) {
        return false;
    }

    /* Map surface region */
    int64_t va = arenaos_syscall2(SYS_SHARED_MAP, SURFACE_SLOT, 1);
    if (va <= 0) {
        return false;
    }
    s_surface_va = (void *)va;

    /* Build Frame::Create */
    uint8_t buf[ADSK_BYTES];
    for (int i = 0; i < ADSK_BYTES; i++) buf[i] = 0;
    buf[0] = 'A'; buf[1] = 'D'; buf[2] = 'S'; buf[3] = 'K';
    buf[4] = 1; /* version */
    buf[5] = 1; /* op Create */
    buf[24] = (uint8_t)(width & 0xff);
    buf[25] = (uint8_t)((width >> 8) & 0xff);
    buf[26] = (uint8_t)(height & 0xff);
    buf[27] = (uint8_t)((height >> 8) & 0xff);

    uint64_t out[3];
    if (exchange(buf, out) != 0 || out[1] == 0) {
        arenaos_syscall1(SYS_SHARED_UNMAP, (uint64_t)s_surface_va);
        s_surface_va = NULL;
        return false;
    }

    s_handle = out[1];
    s_width = width;
    s_height = height;

    /* Set title if provided */
    if (title && title[0]) {
        for (int i = 0; i < ADSK_BYTES; i++) buf[i] = 0;
        buf[0] = 'A'; buf[1] = 'D'; buf[2] = 'S'; buf[3] = 'K';
        buf[4] = 1;
        buf[5] = 4; /* op Title */
        for (int i = 0; i < 8; i++) {
            buf[8 + i] = (uint8_t)((s_handle >> (i * 8)) & 0xff);
        }
        for (int i = 0; i < 32 && title[i]; i++) {
            buf[32 + i] = (uint8_t)title[i];
        }
        exchange(buf, out);
    }

    return true;
}

void adsk_disconnect(void) {
    if (s_handle != 0) {
        uint8_t buf[ADSK_BYTES];
        for (int i = 0; i < ADSK_BYTES; i++) buf[i] = 0;
        buf[0] = 'A'; buf[1] = 'D'; buf[2] = 'S'; buf[3] = 'K';
        buf[4] = 1;
        buf[5] = 11; /* op DestroyWindow */
        for (int i = 0; i < 8; i++) {
            buf[8 + i] = (uint8_t)((s_handle >> (i * 8)) & 0xff);
        }
        uint64_t out[3];
        exchange(buf, out);
        s_handle = 0;
    }

    if (s_surface_va) {
        arenaos_syscall1(SYS_SHARED_UNMAP, (uint64_t)s_surface_va);
        s_surface_va = NULL;
    }
}

bool adsk_damage_full(void) {
    if (s_handle == 0) return false;

    uint8_t buf[ADSK_BYTES];
    for (int i = 0; i < ADSK_BYTES; i++) buf[i] = 0;
    buf[0] = 'A'; buf[1] = 'D'; buf[2] = 'S'; buf[3] = 'K';
    buf[4] = 1;
    buf[5] = 2; /* op Damage */
    for (int i = 0; i < 8; i++) {
        buf[8 + i] = (uint8_t)((s_handle >> (i * 8)) & 0xff);
    }
    buf[16] = 0; /* n = 0 means full window */

    uint64_t out[3];
    return (exchange(buf, out) == 0);
}

bool adsk_damage_rects(int count, const uint16_t rects[][4]) {
    if (s_handle == 0 || count <= 0) return adsk_damage_full();
    if (count > 5) {
        /* ADSK-v1 wire protocol admits at most 5 damage rectangles.
         * Fall back to full-surface damage to avoid silently dropping regions. */
        return adsk_damage_full();
    }

    /* Validate each rectangle against surface bounds */
    for (int i = 0; i < count; i++) {
        uint16_t x = rects[i][0];
        uint16_t y = rects[i][1];
        uint16_t w = rects[i][2];
        uint16_t h = rects[i][3];
        if (w == 0 || h == 0 ||
            ((uint32_t)x + (uint32_t)w > (uint32_t)s_width) ||
            ((uint32_t)y + (uint32_t)h > (uint32_t)s_height)) {
            /* Invalid or out-of-bounds rectangle: fall back to full damage */
            return adsk_damage_full();
        }
    }

    uint8_t buf[ADSK_BYTES];
    for (int i = 0; i < ADSK_BYTES; i++) buf[i] = 0;
    buf[0] = 'A'; buf[1] = 'D'; buf[2] = 'S'; buf[3] = 'K';
    buf[4] = 1;
    buf[5] = 2; /* op Damage */
    for (int i = 0; i < 8; i++) {
        buf[8 + i] = (uint8_t)((s_handle >> (i * 8)) & 0xff);
    }
    buf[16] = (uint8_t)count;

    for (int i = 0; i < count; i++) {
        int base = 24 + i * 8;
        buf[base + 0] = (uint8_t)(rects[i][0] & 0xff);
        buf[base + 1] = (uint8_t)((rects[i][0] >> 8) & 0xff);
        buf[base + 2] = (uint8_t)(rects[i][1] & 0xff);
        buf[base + 3] = (uint8_t)((rects[i][1] >> 8) & 0xff);
        buf[base + 4] = (uint8_t)(rects[i][2] & 0xff);
        buf[base + 5] = (uint8_t)((rects[i][2] >> 8) & 0xff);
        buf[base + 6] = (uint8_t)(rects[i][3] & 0xff);
        buf[base + 7] = (uint8_t)((rects[i][3] >> 8) & 0xff);
    }

    uint64_t out[3];
    return (exchange(buf, out) == 0);
}

bool adsk_poll_event(struct adsk_event *out_event) {
    if (out_event) {
        out_event->type = ADSK_EVT_NONE;
    }
    if (s_handle == 0 || !out_event) return false;

    uint8_t buf[ADSK_BYTES];
    for (int i = 0; i < ADSK_BYTES; i++) buf[i] = 0;
    buf[0] = 'A'; buf[1] = 'D'; buf[2] = 'S'; buf[3] = 'K';
    buf[4] = 1;
    buf[5] = 3; /* op Poll */
    for (int i = 0; i < 8; i++) {
        buf[8 + i] = (uint8_t)((s_handle >> (i * 8)) & 0xff);
    }

    uint64_t out[3];
    if (exchange(buf, out) != 0) {
        return false;
    }

    /* Check if reply is Event (op 5) */
    if (buf[5] == 5) {
        uint8_t ev_kind = buf[28];
        switch (ev_kind) {
            case 1: /* Key */
                out_event->type = ADSK_EVT_KEY;
                out_event->data.key.key = (uint16_t)buf[24] | ((uint16_t)buf[25] << 8);
                return true;
            case 2: /* Pointer */
                out_event->type = ADSK_EVT_POINTER;
                out_event->data.pointer.x = (int32_t)(
                    (uint32_t)buf[16] | ((uint32_t)buf[17] << 8) |
                    ((uint32_t)buf[18] << 16) | ((uint32_t)buf[19] << 24)
                );
                out_event->data.pointer.y = (int32_t)(
                    (uint32_t)buf[20] | ((uint32_t)buf[21] << 8) |
                    ((uint32_t)buf[22] << 16) | ((uint32_t)buf[23] << 24)
                );
                out_event->data.pointer.buttons = buf[29];
                return true;
            case 3: /* Close */
                out_event->type = ADSK_EVT_CLOSE;
                return true;
            case 4: /* Focus */
                out_event->type = ADSK_EVT_FOCUS;
                out_event->data.focus.focused = (buf[29] != 0);
                return true;
            case 5: /* Configure */
                out_event->type = ADSK_EVT_CONFIGURE;
                out_event->data.configure.width = (uint16_t)buf[24] | ((uint16_t)buf[25] << 8);
                out_event->data.configure.height = (uint16_t)buf[26] | ((uint16_t)buf[27] << 8);
                return true;
            case 8: /* Chord */
                out_event->type = ADSK_EVT_CHORD;
                out_event->data.chord.code = (uint16_t)buf[24] | ((uint16_t)buf[25] << 8);
                out_event->data.chord.mods = buf[29];
                return true;
            case 9: /* Wheel */
                out_event->type = ADSK_EVT_WHEEL;
                out_event->data.wheel.x = (int32_t)(
                    (uint32_t)buf[16] | ((uint32_t)buf[17] << 8) |
                    ((uint32_t)buf[18] << 16) | ((uint32_t)buf[19] << 24)
                );
                out_event->data.wheel.y = (int32_t)(
                    (uint32_t)buf[20] | ((uint32_t)buf[21] << 8) |
                    ((uint32_t)buf[22] << 16) | ((uint32_t)buf[23] << 24)
                );
                out_event->data.wheel.delta = (int8_t)buf[29];
                return true;
            default:
                break;
        }
    }

    return false;
}

void *adsk_get_pixels(void) {
    if (!s_surface_va) return NULL;
    return (void *)((uint8_t *)s_surface_va + PIXEL_OFFSET);
}

uint16_t adsk_get_width(void) {
    return s_width;
}

uint16_t adsk_get_height(void) {
    return s_height;
}

uint64_t adsk_get_handle(void) {
    return s_handle;
}
