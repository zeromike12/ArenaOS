/*
 * Hardened buddy allocator over one process-owned VM reservation (C1.4).
 *
 *   - One lazy reservation of HEAP_PAGES (16 MiB). Pages are committed in
 *     64 KiB chunks, only where a block or block header is actually written.
 *     Commit is tracked per chunk, so an uncommitted address is never read.
 *   - Power-of-two blocks from 32 bytes to 16 MiB. Every block starts at an
 *     offset that is a multiple of its size. Each block has a 16-byte header
 *     {tag, level}; the user pointer is header + 16, so it is 16-aligned.
 *   - Ownership is validated BEFORE any metadata read: the pointer must lie
 *     in this reservation, be 16-aligned, sit in a committed chunk, carry a
 *     LIVE tag bound to its own address, have a legal level, and be aligned
 *     to that level. Anything else (foreign, interior, freed, forged) is
 *     counted in bad_frees and ignored. realloc validates the same way and
 *     never frees or copies from an unvalidated pointer.
 *   - Coalescing is bounded: free() merges with its buddy at most
 *     (TOP_LEVEL - level) times, one level at a time.
 *   - Sizes are checked for overflow before rounding. Requests above the
 *     largest block are refused, not truncated.
 *   - Allocation failure (too big, reservation refused, commit refused) returns
 *     NULL. State is changed only after every needed commit has succeeded, so
 *     a refused commit leaves the free lists and headers exactly as they were.
 *   - Thread-safe: one spinlock serializes all allocator state. Contended
 *     acquires yield through arena_backoff().
 *
 * Limits of this check: a header tag is 64 bits of address-bound mixing, not a
 * cryptographic secret. A program that writes the exact tag at an interior
 * address can forge a header, which is a self-inflicted corruption of its own
 * address space, not a privilege boundary. Reuse of an address by a later
 * allocation is indistinguishable from the original pointer (no generations).
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/rt.h"
#include "arena/string.h"
#include "vm.h"

#define HEAP_PAGES 4096u
#define HEAP_BYTES ((uint64_t)HEAP_PAGES * ARENA_PAGE_SIZE)
#define TOP_LEVEL 24u
#define MIN_LEVEL 5u
#define HEADER_BYTES 16u
#define CHUNK_SHIFT 16u
#define CHUNK_BYTES ((uint64_t)1 << CHUNK_SHIFT)
#define CHUNK_PAGES (CHUNK_BYTES / ARENA_PAGE_SIZE)
#define CHUNKS (HEAP_BYTES / CHUNK_BYTES)
#define MAX_PAYLOAD (HEAP_BYTES - HEADER_BYTES)
#define NIL UINT64_MAX

#define LIVE_KIND 0x4C4956455F424C4BULL /* "LIVE_BLK" */
#define FREE_KIND 0x46524545424C4B21ULL /* "FREEBLK!" */

static struct {
    int lock;
    int ready;
    uint64_t slot;
    uintptr_t base;
    uint64_t committed[CHUNKS / 64];
    uint64_t committed_chunks;
    uint64_t new_commits; /* chunks committed by the current call */
    uint64_t free_head[TOP_LEVEL + 1];
    struct arena_heap_stats stats;
} H;

static void lock(void) {
    while (__atomic_exchange_n(&H.lock, 1, __ATOMIC_ACQUIRE)) {
        arena_backoff();
    }
}

static void unlock(void) {
    __atomic_store_n(&H.lock, 0, __ATOMIC_RELEASE);
}

static uint64_t tag_for(uint64_t off, uint64_t kind) {
    uint64_t addr = (uint64_t)H.base + off;
    return kind ^ (addr * 0x9E3779B97F4A7C15ULL) ^ 0x5DEECE66DULL;
}

static int is_committed(uint64_t off) {
    uint64_t chunk = off >> CHUNK_SHIFT;
    return (H.committed[chunk / 64] >> (chunk % 64)) & 1u;
}

/* Commit every chunk covering [off, off + len). Returns 0 or -1; on -1 the
 * chunks committed so far stay committed (harmless) and nothing else changed. */
static int ensure(uint64_t off, uint64_t len) {
    if (len == 0) {
        return 0;
    }
    uint64_t first = off >> CHUNK_SHIFT;
    uint64_t last = (off + len - 1) >> CHUNK_SHIFT;
    for (uint64_t c = first; c <= last; c++) {
        if (is_committed(c << CHUNK_SHIFT)) {
            continue;
        }
        if (arena_vm_commit(H.slot, (uint32_t)(c * CHUNK_PAGES), (uint32_t)CHUNK_PAGES) != 0) {
            return -1;
        }
        H.committed[c / 64] |= (uint64_t)1 << (c % 64);
        H.committed_chunks++;
        H.new_commits++;
    }
    return 0;
}

static uint64_t *hdr(uint64_t off) {
    return (uint64_t *)(H.base + off);
}

static void write_header(uint64_t off, uint64_t kind, uint32_t level) {
    uint64_t *h = hdr(off);
    h[0] = tag_for(off, kind);
    h[1] = level;
}

/* Free-list links live in the payload of a free block (level >= 5 gives 16
 * bytes after the header). Both lie inside a committed chunk. */
static uint64_t *link_next(uint64_t off) {
    return (uint64_t *)(H.base + off + HEADER_BYTES);
}

static uint64_t *link_prev(uint64_t off) {
    return (uint64_t *)(H.base + off + HEADER_BYTES + 8);
}

static void push_free(uint32_t level, uint64_t off) {
    uint64_t head = H.free_head[level];
    *link_next(off) = head;
    *link_prev(off) = NIL;
    if (head != NIL) {
        *link_prev(head) = off;
    }
    H.free_head[level] = off;
}

static void unlink_free(uint32_t level, uint64_t off) {
    uint64_t next = *link_next(off);
    uint64_t prev = *link_prev(off);
    if (prev != NIL) {
        *link_next(prev) = next;
    } else {
        H.free_head[level] = next;
    }
    if (next != NIL) {
        *link_prev(next) = prev;
    }
}

/* Lazily reserve and seed the single top-level free block. */
static int heap_ready(void) {
    if (H.ready) {
        return 0;
    }
    uint64_t slot = 0;
    uintptr_t base = 0;
    if (arena_vm_reserve(HEAP_PAGES, &slot, &base) != 0) {
        return -1;
    }
    if (base == 0 || (base & (ARENA_PAGE_SIZE - 1u)) != 0) {
        (void)arena_vm_release(slot, base, HEAP_PAGES);
        return -1;
    }
    H.slot = slot;
    H.base = base;
    H.stats.reserved_pages = HEAP_PAGES;
    if (ensure(0, HEADER_BYTES) != 0) {
        (void)arena_vm_release(slot, base, HEAP_PAGES);
        H.base = 0;
        return -1;
    }
    /* H is zero-initialised: every list head must start as NIL, not offset 0,
     * or take_block() treats offset 0 as a free block. */
    for (uint32_t l = 0; l <= TOP_LEVEL; l++) {
        H.free_head[l] = NIL;
    }
    write_header(0, FREE_KIND, TOP_LEVEL);
    push_free(TOP_LEVEL, 0);
    H.ready = 1;
    return 0;
}

static uint32_t level_for(size_t n) {
    uint64_t need = (uint64_t)n + HEADER_BYTES;
    uint32_t level = MIN_LEVEL;
    while (((uint64_t)1 << level) < need) {
        level++;
    }
    return level;
}

/* Take a block of exactly `want` from the free lists, splitting as needed.
 * Every commit is performed before any list is touched. Returns the offset,
 * or -1 for no fitting block, or -2 for a refused commit. */
static int64_t take_block(uint32_t want) {
    uint32_t have = want;
    while (have <= TOP_LEVEL && H.free_head[have] == NIL) {
        have++;
    }
    if (have > TOP_LEVEL) {
        return -1;
    }
    uint64_t off = H.free_head[have];
    if (ensure(off, (uint64_t)1 << want) != 0) {
        return -2;
    }
    for (uint32_t k = want; k < have; k++) {
        if (ensure(off + ((uint64_t)1 << k), HEADER_BYTES) != 0) {
            return -2;
        }
    }
    unlink_free(have, off);
    while (have > want) {
        have--;
        uint64_t buddy = off + ((uint64_t)1 << have);
        write_header(buddy, FREE_KIND, have);
        push_free(have, buddy);
    }
    write_header(off, LIVE_KIND, want);
    return (int64_t)off;
}

static void *alloc_locked(size_t n) {
    if (heap_ready() != 0) {
        H.stats.refused++;
        return NULL;
    }
    if ((uint64_t)n > MAX_PAYLOAD) {
        H.stats.refused++;
        return NULL;
    }
    uint32_t level = level_for(n);
    H.new_commits = 0;
    int64_t off = take_block(level);
    if (off < 0) {
        H.stats.refused++;
        return NULL;
    }
    if (H.new_commits == 0) {
        H.stats.reuse_hits++;
    }
    H.stats.live_blocks++;
    H.stats.live_bytes += ((uint64_t)1 << level) - HEADER_BYTES;
    return (void *)(H.base + (uint64_t)off + HEADER_BYTES);
}

/* Ownership validation. Reads metadata only after the address and chunk
 * checks pass. Returns 1 and the block's header offset and level when the
 * pointer is a live block start in this heap. */
static int locate_live(const void *p, uint64_t *off_out, uint32_t *level_out) {
    if (!H.ready || p == NULL) {
        return 0;
    }
    uintptr_t a = (uintptr_t)p;
    if (a < H.base + HEADER_BYTES || a - H.base >= HEAP_BYTES) {
        return 0;
    }
    uint64_t user = a - H.base;
    if ((user & 15u) != 0) {
        return 0;
    }
    uint64_t off = user - HEADER_BYTES;
    if (!is_committed(off)) {
        return 0;
    }
    uint64_t *h = hdr(off);
    uint64_t level = h[1];
    if (level < MIN_LEVEL || level > TOP_LEVEL) {
        return 0;
    }
    if (h[0] != tag_for(off, LIVE_KIND)) {
        return 0;
    }
    if ((off & (((uint64_t)1 << level) - 1)) != 0) {
        return 0;
    }
    *off_out = off;
    *level_out = (uint32_t)level;
    return 1;
}

static void free_locked(uint64_t off, uint32_t level) {
    uint64_t cap = ((uint64_t)1 << level) - HEADER_BYTES;
    H.stats.free_calls++;
    H.stats.live_blocks--;
    H.stats.live_bytes -= cap;
    write_header(off, FREE_KIND, level);
    while (level < TOP_LEVEL) {
        uint64_t buddy = off ^ ((uint64_t)1 << level);
        if (!is_committed(buddy)) {
            break;
        }
        uint64_t *bh = hdr(buddy);
        if (bh[0] != tag_for(buddy, FREE_KIND) || bh[1] != level) {
            break;
        }
        unlink_free(level, buddy);
        if (buddy < off) {
            off = buddy;
        }
        level++;
        write_header(off, FREE_KIND, level);
    }
    push_free(level, off);
}

void *arena_malloc(size_t n) {
    lock();
    void *p = alloc_locked(n);
    unlock();
    return p;
}

void *arena_calloc(size_t count, size_t size) {
    if (count != 0 && size > SIZE_MAX / count) {
        lock();
        H.stats.refused++;
        unlock();
        return NULL;
    }
    size_t n = count * size;
    void *p = arena_malloc(n);
    if (p != NULL) {
        arena_memset(p, 0, n); /* reused blocks are not zeroed by the heap */
    }
    return p;
}

void arena_free(void *p) {
    if (p == NULL) {
        return; /* free(NULL) is a no-op, not a bad free */
    }
    lock();
    uint64_t off = 0;
    uint32_t level = 0;
    if (locate_live(p, &off, &level)) {
        free_locked(off, level);
    } else {
        H.stats.bad_frees++;
    }
    unlock();
}

void *arena_realloc(void *p, size_t n) {
    if (p == NULL) {
        return arena_malloc(n);
    }
    lock();
    uint64_t off = 0;
    uint32_t level = 0;
    if (!locate_live(p, &off, &level)) {
        H.stats.bad_frees++;
        unlock();
        return NULL; /* not ours: nothing is freed, nothing is copied */
    }
    if (n == 0) {
        free_locked(off, level);
        unlock();
        return NULL;
    }
    uint64_t cap = ((uint64_t)1 << level) - HEADER_BYTES;
    if ((uint64_t)n <= cap) {
        unlock();
        return p;
    }
    void *q = alloc_locked(n);
    if (q == NULL) {
        unlock();
        return NULL; /* old block untouched and still valid */
    }
    arena_memcpy(q, p, (size_t)cap);
    free_locked(off, level);
    unlock();
    return q;
}

void arena_heap_stats(struct arena_heap_stats *out) {
    lock();
    *out = H.stats;
    out->committed_pages = H.committed_chunks * CHUNK_PAGES;
    unlock();
}
