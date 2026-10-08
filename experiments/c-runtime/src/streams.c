/*
 * ASTR stream rings and granted stdio (C1.1).
 *
 * Wire format (userspace/arena-runtime/src/streams.rs, unchanged):
 *   page: 64-byte header {magic "ASTR", version 1, channels 3, capacity 768,
 *         reserved[48] = 0}, then three 832-byte rings at 64 + 832 * channel.
 *   ring: 64-byte control {write u32 @0, read u32 @4, state u32 @8, pad}, then
 *         768 data bytes at ring + 64. state bits: WRITER_CLOSED 1,
 *         READER_CLOSED 2.
 * Channel 0 is stdin (the desktop writes, the app reads), 1 stdout and 2 stderr
 * (the app writes, the desktop reads).
 *
 * Protocol rules (identical to the Rust runtime):
 *   - SPSC: one producer and one consumer per ring. The producer publishes
 *     bytes before its release-store to write; the consumer acquire-loads write
 *     before reading.
 *   - write: a short count is valid; a full ring returns WOULD_BLOCK; a closed
 *     reader returns BROKEN_PIPE.
 *   - read: 0 means EOF only after WRITER_CLOSED with the ring drained; an
 *     empty open ring returns WOULD_BLOCK.
 *
 * Granted stdio: fd 0/1/2 are local indices onto the StandardStreamSet and
 * StreamWake capabilities from the verified startup record. They are not
 * authority. Backpressure waits on the granted private Notification
 * (BADGE_STREAM_READY from the desktop). Writes wake the desktop via the
 * WRITE-only StreamWake notification (badge 1 << 5).
 *
 * Known protocol limit, kept for wire compatibility: the counters are u32 and
 * index the ring with `counter % 768`. The index is not continuous across the
 * 2^32 wrap, because 2^32 is not a multiple of 768. Traffic that crosses the
 * wrap is misordered. Documented in docs/compat/C1-FINAL-REPORT.md (finding
 * F-C1-RING-WRAP) and reproduced by the host test. Fixing it changes the wire
 * format, so it is a proposed ADR, not a change made here.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "arena/rt.h"
#include "arena/string.h"
#include "internal.h"

#define HEADER_BYTES 64u
#define RING_STRIDE 832u
#define CONTROL_BYTES 64u
#define WRITER_CLOSED 1u
#define READER_CLOSED 2u

#define NO_SLOT 0xFFFFFFFFFFFFFFFFULL

static uint8_t *granted_page;
static uint64_t wake_slot = NO_SLOT;
static uint64_t wait_slot = NO_SLOT;
static int stdio_ready;

static uint8_t *ring_at(uint8_t *page, unsigned channel) {
    return page + HEADER_BYTES + (size_t)channel * RING_STRIDE;
}

static const uint8_t *ring_at_c(const uint8_t *page, unsigned channel) {
    return page + HEADER_BYTES + (size_t)channel * RING_STRIDE;
}

static uint32_t load_u32(const uint8_t *at, int order) {
    return __atomic_load_n((const uint32_t *)(const void *)at, order);
}

static void store_u32(uint8_t *at, uint32_t v, int order) {
    __atomic_store_n((uint32_t *)(void *)at, v, order);
}

static uint32_t fetch_or_state(uint8_t *ring, uint32_t bits) {
    return __atomic_fetch_or((uint32_t *)(void *)(ring + 8), bits, __ATOMIC_ACQ_REL);
}

int arena_ring_page_check(const void *page_v) {
    const uint8_t *page = (const uint8_t *)page_v;
    if (page == NULL) {
        return ARENA_E_INVALID;
    }
    if (arena_memcmp(page, "ASTR", 4) != 0) {
        return ARENA_E_CORRUPT;
    }
    if (load_u32(page + 4, __ATOMIC_RELAXED) != ARENA_STREAM_VERSION ||
        load_u32(page + 8, __ATOMIC_RELAXED) != ARENA_STREAM_CHANNELS ||
        load_u32(page + 12, __ATOMIC_RELAXED) != ARENA_STREAM_CAPACITY) {
        return ARENA_E_CORRUPT;
    }
    for (size_t i = 16; i < HEADER_BYTES; i++) {
        if (page[i] != 0) {
            return ARENA_E_CORRUPT;
        }
    }
    return 0;
}

int arena_ring_page_init(void *page_v) {
    uint8_t *page = (uint8_t *)page_v;
    if (page == NULL) {
        return ARENA_E_INVALID;
    }
    arena_memset(page, 0, ARENA_STREAM_PAGE_BYTES);
    arena_memcpy(page, "ASTR", 4);
    store_u32(page + 4, ARENA_STREAM_VERSION, __ATOMIC_RELAXED);
    store_u32(page + 8, ARENA_STREAM_CHANNELS, __ATOMIC_RELAXED);
    store_u32(page + 12, ARENA_STREAM_CAPACITY, __ATOMIC_RELAXED);
    return 0;
}

long arena_ring_write_some(void *page_v, unsigned channel, const void *buf, size_t len) {
    uint8_t *page = (uint8_t *)page_v;
    if (page == NULL || channel >= ARENA_STREAM_CHANNELS || (buf == NULL && len != 0)) {
        return ARENA_E_INVALID;
    }
    if (len == 0) {
        return 0;
    }
    uint8_t *ring = ring_at(page, channel);
    uint32_t state = load_u32(ring + 8, __ATOMIC_ACQUIRE);
    if (state & READER_CLOSED) {
        return ARENA_E_BROKEN_PIPE;
    }
    if (state & WRITER_CLOSED) {
        return ARENA_E_CLOSED;
    }
    uint32_t head = load_u32(ring + 0, __ATOMIC_RELAXED);
    uint32_t tail = load_u32(ring + 4, __ATOMIC_ACQUIRE);
    uint32_t occupied = head - tail;
    if (occupied > ARENA_STREAM_CAPACITY) {
        return ARENA_E_CORRUPT;
    }
    size_t available = ARENA_STREAM_CAPACITY - occupied;
    if (available == 0) {
        return ARENA_E_WOULD_BLOCK;
    }
    size_t count = len < available ? len : available;
    size_t start = head % ARENA_STREAM_CAPACITY;
    size_t first = count < ARENA_STREAM_CAPACITY - start ? count : ARENA_STREAM_CAPACITY - start;
    uint8_t *data = ring + CONTROL_BYTES;
    arena_memcpy(data + start, buf, first);
    if (count > first) {
        arena_memcpy(data, (const uint8_t *)buf + first, count - first);
    }
    store_u32(ring + 0, head + (uint32_t)count, __ATOMIC_RELEASE);
    return (long)count;
}

long arena_ring_read_some(void *page_v, unsigned channel, void *buf, size_t len) {
    uint8_t *page = (uint8_t *)page_v;
    if (page == NULL || channel >= ARENA_STREAM_CHANNELS || (buf == NULL && len != 0)) {
        return ARENA_E_INVALID;
    }
    if (len == 0) {
        return 0;
    }
    uint8_t *ring = ring_at(page, channel);
    uint32_t head = load_u32(ring + 0, __ATOMIC_ACQUIRE);
    uint32_t tail = load_u32(ring + 4, __ATOMIC_RELAXED);
    uint32_t available = head - tail;
    if (available > ARENA_STREAM_CAPACITY) {
        return ARENA_E_CORRUPT;
    }
    if (available == 0) {
        uint32_t state = load_u32(ring + 8, __ATOMIC_ACQUIRE);
        if (state & WRITER_CLOSED) {
            return 0; /* EOF: writer closed and drained */
        }
        if (state & READER_CLOSED) {
            return ARENA_E_CLOSED;
        }
        return ARENA_E_WOULD_BLOCK;
    }
    if (load_u32(ring + 8, __ATOMIC_RELAXED) & READER_CLOSED) {
        return ARENA_E_CLOSED;
    }
    size_t count = len < available ? len : available;
    size_t start = tail % ARENA_STREAM_CAPACITY;
    size_t first = count < ARENA_STREAM_CAPACITY - start ? count : ARENA_STREAM_CAPACITY - start;
    const uint8_t *data = ring + CONTROL_BYTES;
    arena_memcpy(buf, data + start, first);
    if (count > first) {
        arena_memcpy((uint8_t *)buf + first, data, count - first);
    }
    store_u32(ring + 4, tail + (uint32_t)count, __ATOMIC_RELEASE);
    return (long)count;
}

int arena_ring_writer_closed(const void *page_v, unsigned channel) {
    const uint8_t *page = (const uint8_t *)page_v;
    if (page == NULL || channel >= ARENA_STREAM_CHANNELS) {
        return 0;
    }
    return (load_u32(ring_at_c(page, channel) + 8, __ATOMIC_ACQUIRE) & WRITER_CLOSED) != 0;
}

void arena_ring_close_writer(void *page_v, unsigned channel) {
    uint8_t *page = (uint8_t *)page_v;
    if (page != NULL && channel < ARENA_STREAM_CHANNELS) {
        (void)fetch_or_state(ring_at(page, channel), WRITER_CLOSED);
    }
}

void arena_ring_close_reader(void *page_v, unsigned channel) {
    uint8_t *page = (uint8_t *)page_v;
    if (page != NULL && channel < ARENA_STREAM_CHANNELS) {
        (void)fetch_or_state(ring_at(page, channel), READER_CLOSED);
    }
}

/* --- granted stdio ----------------------------------------------------- */

static int chan_of_fd(int fd) {
    return fd == 0 ? 0 : (fd == 1 ? 1 : (fd == 2 ? 2 : -1));
}

int arena_stdio_init(void) {
    if (stdio_ready) {
        return 0;
    }
    const arena_startup_t *s = arena_startup();
    if (!s->present || (s->flags & ARENA_ARST_FLAG_STANDARD_STREAMS) == 0 ||
        s->stream_set == ARENA_ARST_NONE || s->stream_wake == ARENA_ARST_NONE) {
        return ARENA_E_NO_STREAMS;
    }
    const arena_cap_desc_t *set = &s->caps[s->stream_set];
    const arena_cap_desc_t *wake = &s->caps[s->stream_wake];
    if (set->kind != ARENA_CAP_KIND_SHARED_REGION || set->rights != (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE) ||
        wake->kind != ARENA_CAP_KIND_NOTIFICATION || wake->rights != ARENA_RIGHT_WRITE) {
        return ARENA_E_STARTUP;
    }
    int64_t pages = arena_syscall6(ARENA_SYS_SHARED_PAGES, set->slot, 0, 0, 0, 0, 0);
    if (pages != 1) {
        return ARENA_E_STARTUP;
    }
    int64_t va = arena_syscall6(ARENA_SYS_SHARED_MAP, set->slot, 1, 0, 0, 0, 0);
    if (va <= 0 || ((uint64_t)va & (ARENA_PAGE_SIZE - 1u)) != 0) {
        return ARENA_E_STARTUP;
    }
    if (arena_ring_page_check((const void *)(uintptr_t)va) != 0) {
        (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP, (uint64_t)va, 0, 0, 0, 0, 0);
        return ARENA_E_STARTUP;
    }
    granted_page = (uint8_t *)(uintptr_t)va;
    wake_slot = wake->slot;
    wait_slot = s->notification == ARENA_ARST_NONE ? NO_SLOT : s->caps[s->notification].slot;
    stdio_ready = 1;
    return 0;
}

long arena_stream_write_some(int fd, const void *buf, size_t len) {
    if (!stdio_ready) {
        return ARENA_E_NO_STREAMS;
    }
    if (fd != 1 && fd != 2) {
        return ARENA_E_INVALID;
    }
    long n = arena_ring_write_some(granted_page, (unsigned)fd, buf, len);
    if (n > 0) {
        /* Wake the desktop's event loop to drain. The badge is a hint; the
         * WRITE-only cap is the authority. A failed hint leaves the bytes in
         * the ring, so it is not reported as a write failure. */
        (void)arena_syscall6(ARENA_SYS_NOTIFY, wake_slot, ARENA_STREAM_WAKE_BADGE, 0, 0, 0, 0);
    }
    return n;
}

long arena_stream_read_some(int fd, void *buf, size_t len) {
    if (!stdio_ready) {
        return ARENA_E_NO_STREAMS;
    }
    if (fd != 0) {
        return ARENA_E_INVALID;
    }
    return arena_ring_read_some(granted_page, 0, buf, len);
}

/* Block until the granted notification delivers any badge. The desktop sends
 * BADGE_STREAM_READY after it drains output or enqueues input. A pending badge
 * is kept, so a wake that arrives before the wait is not lost. */
static long wait_for_peer(void) {
    if (wait_slot == NO_SLOT) {
        return ARENA_E_NO_WAITER;
    }
    int64_t badge = arena_syscall6(ARENA_SYS_WAIT, wait_slot, 0, 0, 0, 0, 0);
    return badge < 0 ? (long)badge : 0;
}

long arena_stream_write(int fd, const void *buf, size_t len) {
    const unsigned char *p = (const unsigned char *)buf;
    size_t done = 0;
    while (done < len) {
        long n = arena_stream_write_some(fd, p + done, len - done);
        if (n > 0) {
            done += (size_t)n;
            continue;
        }
        if (n != ARENA_E_WOULD_BLOCK) {
            return n;
        }
        long w = wait_for_peer();
        if (w < 0) {
            return w;
        }
    }
    return (long)done;
}

long arena_stream_read(int fd, void *buf, size_t len) {
    if (len == 0) {
        return 0;
    }
    for (;;) {
        long n = arena_stream_read_some(fd, buf, len);
        if (n != ARENA_E_WOULD_BLOCK) {
            return n;
        }
        long w = wait_for_peer();
        if (w < 0) {
            return w;
        }
    }
}

int arena_stream_close(int fd) {
    if (!stdio_ready) {
        return ARENA_E_NO_STREAMS;
    }
    int chan = chan_of_fd(fd);
    if (chan < 0) {
        return ARENA_E_INVALID;
    }
    if (chan == 0) {
        arena_ring_close_reader(granted_page, 0);
    } else {
        arena_ring_close_writer(granted_page, (unsigned)chan);
    }
    return 0;
}
