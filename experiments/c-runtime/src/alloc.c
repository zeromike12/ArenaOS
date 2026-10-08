/*
 * Size-class allocator over one process-owned VM reservation (prototype).
 *
 *   - One reservation of HEAP_RESERVE_PAGES (4 MiB) is taken on first use.
 *   - Pages are committed lazily, COMMIT_CHUNK_PAGES (64 KiB) at a time, as
 *     the bump pointer advances. Freshly committed pages are zero (kernel
 *     guarantee, ADR-0095), but reused blocks are not: calloc always clears.
 *   - Requests are rounded up to a power-of-two capacity class from 16 B to
 *     1 MiB. Freed blocks go on a per-class LIFO free list and are reused
 *     only by the same class. There is NO coalescing: this trades up to 2x
 *     internal fragmentation and no cross-class reuse for a design that is
 *     small enough to verify exhaustively on the host.
 *   - Every block carries a 16-byte header {capacity, tag}. free() checks
 *     the tag and the address range; a pointer that is not live is counted
 *     and ignored, never trusted.
 *   - Allocation failure (too big, reservation exhausted, commit refused by
 *     the kernel) returns NULL and is counted. No abort path.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/rt.h"
#include "arena/string.h"
#include "vm.h"

#define HEAP_RESERVE_PAGES 1024u
#define COMMIT_CHUNK_PAGES 16u
#define PAGE_BYTES 4096u
#define HEADER_BYTES 16u
#define MIN_CLASS 4u  /* 2^4 = 16 bytes */
#define MAX_CLASS 20u /* 2^20 = 1 MiB */
#define NCLASSES (MAX_CLASS + 1u)

#define TAG_LIVE UINT64_C(0xA11CA110C0DE0001)
#define TAG_FREE UINT64_C(0xA11CF4EE00000002)

struct header {
    uint64_t capacity; /* power of two, MIN..2^MAX_CLASS */
    uint64_t tag;
};

struct free_block {
    struct free_block *next; /* lives in the payload of a free block */
};

static struct {
    int initialised;
    int failed; /* reservation failed: every allocation is refused */
    uint64_t slot;
    uintptr_t base;
    uintptr_t bump;
    uintptr_t commit_end; /* end of committed bytes */
    uintptr_t limit;      /* end of reservation */
    struct free_block *free_head[NCLASSES];
    struct arena_heap_stats st;
} H;

static int heap_init(void) {
    if (H.initialised) {
        return !H.failed;
    }
    H.initialised = 1;
    uintptr_t base = 0;
    uint64_t slot = 0;
    if (arena_vm_reserve(HEAP_RESERVE_PAGES, &slot, &base) != 0) {
        H.failed = 1;
        return 0;
    }
    H.slot = slot;
    H.base = base;
    H.bump = base;
    H.commit_end = base;
    H.limit = base + (uintptr_t)HEAP_RESERVE_PAGES * PAGE_BYTES;
    H.st.reserved_pages = HEAP_RESERVE_PAGES;
    return 1;
}

/* Ensure committed bytes cover [H.base, upto). Commits whole chunks, clamped
 * to the reservation. Returns 0 on success. */
static int heap_commit_to(uintptr_t upto) {
    if (upto <= H.commit_end) {
        return 0;
    }
    uintptr_t need = upto - H.commit_end;
    uint32_t pages = (uint32_t)((need + PAGE_BYTES - 1u) / PAGE_BYTES);
    uint32_t chunk_pages = (uint32_t)(((pages + COMMIT_CHUNK_PAGES - 1u) / COMMIT_CHUNK_PAGES) *
                                      COMMIT_CHUNK_PAGES);
    uint32_t committed_pages = (uint32_t)((H.commit_end - H.base) / PAGE_BYTES);
    uint32_t room = HEAP_RESERVE_PAGES - committed_pages;
    if (chunk_pages > room) {
        chunk_pages = room;
    }
    if (chunk_pages < pages) {
        return -1;
    }
    if (arena_vm_commit(H.slot, committed_pages, chunk_pages) != 0) {
        return -1;
    }
    H.commit_end += (uintptr_t)chunk_pages * PAGE_BYTES;
    H.st.committed_pages = (H.commit_end - H.base) / PAGE_BYTES;
    return 0;
}

static unsigned class_of(size_t n) {
    uint64_t want = n < (1u << MIN_CLASS) ? (1u << MIN_CLASS) : (uint64_t)n;
    unsigned k = MIN_CLASS;
    while ((UINT64_C(1) << k) < want) {
        k++;
    }
    return k;
}

void *arena_malloc(size_t n) {
    H.st.alloc_calls++;
    if (n > (size_t)(UINT64_C(1) << MAX_CLASS) || !heap_init()) {
        H.st.refused++;
        return NULL;
    }
    unsigned k = class_of(n);
    if (H.free_head[k] != NULL) {
        struct free_block *b = H.free_head[k];
        H.free_head[k] = b->next;
        struct header *h = (struct header *)((uintptr_t)b - HEADER_BYTES);
        h->tag = TAG_LIVE;
        H.st.reuse_hits++;
        H.st.live_blocks++;
        H.st.live_bytes += h->capacity;
        return (void *)b;
    }
    uint64_t cap = UINT64_C(1) << k;
    uintptr_t need = (uintptr_t)(HEADER_BYTES + cap);
    if (H.bump > H.limit || need > H.limit - H.bump) {
        H.st.refused++;
        return NULL;
    }
    if (heap_commit_to(H.bump + need) != 0) {
        H.st.refused++;
        return NULL;
    }
    struct header *h = (struct header *)H.bump;
    h->capacity = cap;
    h->tag = TAG_LIVE;
    H.bump += need;
    H.st.live_blocks++;
    H.st.live_bytes += cap;
    return (void *)((uintptr_t)h + HEADER_BYTES);
}

void arena_free(void *p) {
    if (p == NULL) {
        return;
    }
    H.st.free_calls++;
    uintptr_t addr = (uintptr_t)p;
    if (!H.initialised || H.failed || addr < H.base + HEADER_BYTES || addr >= H.bump) {
        H.st.bad_frees++;
        return;
    }
    struct header *h = (struct header *)(addr - HEADER_BYTES);
    unsigned k = 0;
    while (k <= MAX_CLASS && (UINT64_C(1) << k) != h->capacity) {
        k++;
    }
    if (h->tag != TAG_LIVE || k < MIN_CLASS || k > MAX_CLASS) {
        H.st.bad_frees++; /* double free, wild pointer, or corrupted header */
        return;
    }
    h->tag = TAG_FREE;
    struct free_block *b = (struct free_block *)p;
    b->next = H.free_head[k];
    H.free_head[k] = b;
    H.st.live_blocks--;
    H.st.live_bytes -= h->capacity;
}

void *arena_calloc(size_t count, size_t size) {
    if (size != 0 && count > (size_t)-1 / size) {
        H.st.alloc_calls++;
        H.st.refused++;
        return NULL;
    }
    size_t total = count * size;
    void *p = arena_malloc(total);
    if (p != NULL) {
        arena_memset(p, 0, total);
    }
    return p;
}

void *arena_realloc(void *p, size_t n) {
    if (p == NULL) {
        return arena_malloc(n);
    }
    if (n == 0) {
        arena_free(p);
        return NULL;
    }
    struct header *h = (struct header *)((uintptr_t)p - HEADER_BYTES);
    if (h->tag == TAG_LIVE && h->capacity >= n) {
        return p;
    }
    void *q = arena_malloc(n);
    if (q == NULL) {
        return NULL; /* original block untouched */
    }
    if (h->tag == TAG_LIVE) {
        arena_memcpy(q, p, (size_t)h->capacity);
    }
    arena_free(p);
    return q;
}

void arena_heap_stats(struct arena_heap_stats *out) {
    if (out != NULL) {
        *out = H.st;
        if (!H.initialised) {
            out->reserved_pages = HEAP_RESERVE_PAGES;
        }
    }
}
