/*
  ArenaOS Freestanding Memory Allocator (Milestone G2.2 Hardened Temporary Scaffolding)
  Notice: This file provides minimal, safe heap allocation scaffolding strictly scoped
  to Milestone G2 graphics experimentation pending Arena 2's general C runtime (C2).
  It is not a general C runtime allocator and is not exported outside experimental/.
*/

#include "arenaos_syscalls.h"

#define HEAP_SIZE (128 * 1024) /* 128 KiB static heap */
#define ARENA_MEM_MAGIC 0x4152454E414D454DULL /* "ARENAMEM" */

static uint8_t s_heap[HEAP_SIZE] __attribute__((aligned(16)));

/*
 * BlockHeader is exactly 32 bytes (16-byte aligned).
 * This guarantees that (header + 1), which is header + 32,
 * is ALWAYS strictly 16-byte aligned whenever header is 16-byte aligned.
 */
typedef struct BlockHeader {
    uint64_t magic;            /* ARENA_MEM_MAGIC */
    size_t size;               /* Payload capacity in bytes (multiple of 16) */
    uint32_t is_free;          /* 1 if free, 0 if allocated */
    uint32_t pad;              /* Explicit alignment padding */
    struct BlockHeader *next;  /* Next block in heap list */
} BlockHeader;

#define BLOCK_HEADER_SIZE (sizeof(BlockHeader))

static BlockHeader *s_free_list = NULL;
static size_t s_total_allocated = 0;
static size_t s_peak_allocated = 0;
static size_t s_alloc_count = 0;
static size_t s_bad_frees = 0;
static int s_mem_lock = 0;

static inline void mem_lock(void) {
    while (__atomic_exchange_n(&s_mem_lock, 1, __ATOMIC_ACQUIRE)) {
        __asm__ volatile("pause" ::: "memory");
    }
}

static inline void mem_unlock(void) {
    __atomic_store_n(&s_mem_lock, 0, __ATOMIC_RELEASE);
}

void arenaos_memory_init(void) {
    mem_lock();
    s_free_list = (BlockHeader *)s_heap;
    s_free_list->magic = ARENA_MEM_MAGIC;
    s_free_list->size = HEAP_SIZE - BLOCK_HEADER_SIZE;
    s_free_list->is_free = 1;
    s_free_list->pad = 0;
    s_free_list->next = NULL;
    s_total_allocated = 0;
    s_peak_allocated = 0;
    s_alloc_count = 0;
    s_bad_frees = 0;
    mem_unlock();
}

static int is_valid_pointer(const void *ptr, BlockHeader **out_hdr) {
    if (!ptr) return 0;
    uintptr_t addr = (uintptr_t)ptr;

    /* User pointer must be 16-byte aligned */
    if ((addr & 15u) != 0) {
        return 0;
    }

    /* User pointer must be inside heap boundaries with room for header */
    uintptr_t heap_start = (uintptr_t)s_heap;
    uintptr_t heap_end = heap_start + HEAP_SIZE;
    if (addr < heap_start + BLOCK_HEADER_SIZE || addr >= heap_end) {
        return 0;
    }

    BlockHeader *hdr = (BlockHeader *)(addr - BLOCK_HEADER_SIZE);
    if (hdr->magic != ARENA_MEM_MAGIC) {
        return 0;
    }
    if (hdr->size == 0 || hdr->size > (HEAP_SIZE - BLOCK_HEADER_SIZE)) {
        return 0;
    }
    if ((uintptr_t)hdr + BLOCK_HEADER_SIZE + hdr->size > heap_end) {
        return 0;
    }

    if (out_hdr) {
        *out_hdr = hdr;
    }
    return 1;
}

void *arenaos_malloc(size_t size) {
    if (size == 0) return NULL;
    if (size > (HEAP_SIZE - BLOCK_HEADER_SIZE)) {
        return NULL; /* Overflow / exceeds total heap */
    }

    mem_lock();
    if (!s_free_list) {
        /* Lazy initialization */
        s_free_list = (BlockHeader *)s_heap;
        s_free_list->magic = ARENA_MEM_MAGIC;
        s_free_list->size = HEAP_SIZE - BLOCK_HEADER_SIZE;
        s_free_list->is_free = 1;
        s_free_list->pad = 0;
        s_free_list->next = NULL;
    }

    /* 16-byte align size */
    size = (size + 15) & ~((size_t)15);

    BlockHeader *curr = s_free_list;
    while (curr) {
        if (curr->magic != ARENA_MEM_MAGIC) {
            /* Corrupted header detected: fail closed */
            mem_unlock();
            return NULL;
        }

        if (curr->is_free && curr->size >= size) {
            /* Can we split this block? We need room for new header (32 B) + at least 16 B payload */
            if (curr->size >= size + BLOCK_HEADER_SIZE + 16) {
                BlockHeader *new_block = (BlockHeader *)((uint8_t *)curr + BLOCK_HEADER_SIZE + size);
                new_block->magic = ARENA_MEM_MAGIC;
                new_block->size = curr->size - size - BLOCK_HEADER_SIZE;
                new_block->is_free = 1;
                new_block->pad = 0;
                new_block->next = curr->next;

                curr->size = size;
                curr->next = new_block;
            }
            curr->is_free = 0;
            s_total_allocated += curr->size;
            if (s_total_allocated > s_peak_allocated) {
                s_peak_allocated = s_total_allocated;
            }
            s_alloc_count++;
            mem_unlock();
            return (void *)((uint8_t *)curr + BLOCK_HEADER_SIZE);
        }
        curr = curr->next;
    }

    /* Out of memory */
    mem_unlock();
    return NULL;
}

void arenaos_free(void *ptr) {
    if (!ptr) return;

    mem_lock();
    BlockHeader *hdr = NULL;
    if (!is_valid_pointer(ptr, &hdr)) {
        s_bad_frees++;
        mem_unlock();
        return;
    }

    if (hdr->is_free) {
        /* Double free detected */
        s_bad_frees++;
        mem_unlock();
        return;
    }

    hdr->is_free = 1;
    if (s_total_allocated >= hdr->size) {
        s_total_allocated -= hdr->size;
    }

    /* Coalesce adjacent free blocks */
    BlockHeader *curr = s_free_list;
    while (curr && curr->next) {
        if (curr->is_free && curr->next->is_free) {
            curr->size += BLOCK_HEADER_SIZE + curr->next->size;
            curr->next = curr->next->next;
        } else {
            curr = curr->next;
        }
    }
    mem_unlock();
}

void *arenaos_calloc(size_t nmemb, size_t size) {
    if (nmemb != 0 && size > (size_t)-1 / nmemb) {
        return NULL; /* Integer multiplication overflow */
    }
    size_t total = nmemb * size;
    void *p = arenaos_malloc(total);
    if (p) {
        uint8_t *b = (uint8_t *)p;
        for (size_t i = 0; i < total; i++) {
            b[i] = 0;
        }
    }
    return p;
}

void *arenaos_realloc(void *ptr, size_t size) {
    if (!ptr) {
        return arenaos_malloc(size);
    }
    if (size == 0) {
        arenaos_free(ptr);
        return NULL;
    }
    if (size > (HEAP_SIZE - BLOCK_HEADER_SIZE)) {
        return NULL; /* Overflow / excessive request */
    }

    mem_lock();
    BlockHeader *hdr = NULL;
    if (!is_valid_pointer(ptr, &hdr)) {
        s_bad_frees++;
        mem_unlock();
        return NULL; /* Untrusted pointer: refuse without mutating heap */
    }

    if (hdr->is_free) {
        s_bad_frees++;
        mem_unlock();
        return NULL;
    }

    size_t aligned_size = (size + 15) & ~((size_t)15);
    if (hdr->size >= aligned_size) {
        /* Current block capacity is already sufficient */
        mem_unlock();
        return ptr;
    }
    mem_unlock();

    /* Allocate larger replacement block */
    void *new_p = arenaos_malloc(size);
    if (!new_p) {
        /* Allocation failed: original block remains untouched and valid */
        return NULL;
    }

    /* Copy existing payload into new block */
    size_t copy_bytes = (hdr->size < size) ? hdr->size : size;
    uint8_t *src = (uint8_t *)ptr;
    uint8_t *dst = (uint8_t *)new_p;
    for (size_t i = 0; i < copy_bytes; i++) {
        dst[i] = src[i];
    }

    arenaos_free(ptr);
    return new_p;
}

size_t arenaos_mem_peak(void) {
    return s_peak_allocated;
}

size_t arenaos_mem_current(void) {
    return s_total_allocated;
}

size_t arenaos_mem_bad_frees(void) {
    return s_bad_frees;
}

#ifndef ARENAOS_HOST_TEST
/* Freestanding string and memory operations (weakly defined to avoid collisions) */
__attribute__((weak)) void *memcpy(void *dest, const void *src, size_t n) {
    uint8_t *d = (uint8_t *)dest;
    const uint8_t *s = (const uint8_t *)src;
    for (size_t i = 0; i < n; i++) {
        d[i] = s[i];
    }
    return dest;
}

__attribute__((weak)) void *memset(void *s, int c, size_t n) {
    uint8_t *p = (uint8_t *)s;
    for (size_t i = 0; i < n; i++) {
        p[i] = (uint8_t)c;
    }
    return s;
}

__attribute__((weak)) void *memmove(void *dest, const void *src, size_t n) {
    uint8_t *d = (uint8_t *)dest;
    const uint8_t *s = (const uint8_t *)src;
    if (d < s) {
        for (size_t i = 0; i < n; i++) {
            d[i] = s[i];
        }
    } else if (d > s) {
        for (size_t i = n; i > 0; i--) {
            d[i - 1] = s[i - 1];
        }
    }
    return dest;
}

__attribute__((weak)) int memcmp(const void *s1, const void *s2, size_t n) {
    const uint8_t *p1 = (const uint8_t *)s1;
    const uint8_t *p2 = (const uint8_t *)s2;
    for (size_t i = 0; i < n; i++) {
        if (p1[i] != p2[i]) {
            return p1[i] - p2[i];
        }
    }
    return 0;
}

__attribute__((weak)) size_t strlen(const char *s) {
    size_t len = 0;
    while (s && s[len]) {
        len++;
    }
    return len;
}

__attribute__((weak)) int strcmp(const char *s1, const char *s2) {
    while (*s1 && (*s1 == *s2)) {
        s1++;
        s2++;
    }
    return *(const unsigned char *)s1 - *(const unsigned char *)s2;
}

__attribute__((weak)) int strncmp(const char *s1, const char *s2, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (s1[i] != s2[i] || s1[i] == '\0') {
            return (unsigned char)s1[i] - (unsigned char)s2[i];
        }
    }
    return 0;
}

#ifndef ARENAOS_HOST_TEST
void *malloc(size_t size) { return arenaos_malloc(size); }
void free(void *ptr) { arenaos_free(ptr); }
void *calloc(size_t nmemb, size_t size) { return arenaos_calloc(nmemb, size); }
void *realloc(void *ptr, size_t size) { return arenaos_realloc(ptr, size); }
#endif

#endif /* !ARENAOS_HOST_TEST */
