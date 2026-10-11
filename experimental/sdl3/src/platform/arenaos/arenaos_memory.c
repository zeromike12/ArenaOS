/*
  ArenaOS Freestanding Memory Allocator & Runtime Memory Primitives
  Hardened allocator with 32-byte 16-aligned header, ownership validation,
  double-free detection, and thread-safe atomic reallocation.
*/

#include <stddef.h>
#include <stdint.h>

#define ARENA_MEM_MAGIC 0x4152454E414D454DULL /* "ARENAMEM" */

/*
 * BlockHeader: exactly 32 bytes (multiple of 16).
 * This guarantees that (header + 32) is always 16-byte aligned.
 */
typedef struct BlockHeader {
    uint64_t magic;           /* 8 bytes: ARENA_MEM_MAGIC */
    size_t size;              /* 8 bytes: usable payload size in bytes */
    uint32_t is_free;         /* 4 bytes: 1 if free, 0 if allocated */
    uint32_t pad;             /* 4 bytes: alignment padding */
    struct BlockHeader *next; /* 8 bytes: next block pointer */
} BlockHeader;

#define BLOCK_HEADER_SIZE sizeof(BlockHeader) /* 32 bytes */
#define HEAP_SIZE (128 * 1024)                /* 128 KiB heap (32 load pages) */

static uint8_t s_heap[HEAP_SIZE] __attribute__((aligned(16)));
static BlockHeader *s_free_list = NULL;
static int s_initialized = 0;
static volatile int s_lock = 0;

/* Statistics for verification */
static size_t s_total_allocated = 0;
static size_t s_peak_allocated = 0;
static uint64_t s_alloc_count = 0;
static uint64_t s_bad_frees = 0;

static inline void mem_lock(void) {
    while (__atomic_test_and_set(&s_lock, __ATOMIC_ACQUIRE)) {
#if defined(__x86_64__)
        __asm__ volatile ("pause");
#endif
    }
}

static inline void mem_unlock(void) {
    __atomic_clear(&s_lock, __ATOMIC_RELEASE);
}

void arenaos_memory_init(void) {
    mem_lock();
    if (!s_initialized) {
        s_free_list = (BlockHeader *)s_heap;
        s_free_list->magic = ARENA_MEM_MAGIC;
        s_free_list->size = HEAP_SIZE - BLOCK_HEADER_SIZE;
        s_free_list->is_free = 1;
        s_free_list->pad = 0;
        s_free_list->next = NULL;
        s_initialized = 1;
    }
    mem_unlock();
}

static int is_valid_pointer(const void *ptr, BlockHeader **out_hdr) {
    if (!ptr) return 0;
    uintptr_t p = (uintptr_t)ptr;
    uintptr_t heap_start = (uintptr_t)s_heap;
    uintptr_t heap_end = heap_start + HEAP_SIZE;

    if (p < (heap_start + BLOCK_HEADER_SIZE) || p >= heap_end) {
        return 0; /* Outside heap boundary */
    }

    if ((p & 15) != 0) {
        return 0; /* Not 16-byte aligned */
    }

    BlockHeader *hdr = (BlockHeader *)(p - BLOCK_HEADER_SIZE);
    if (hdr->magic != ARENA_MEM_MAGIC) {
        return 0; /* Missing magic ownership tag */
    }

    if (hdr->size == 0 || (uintptr_t)hdr + BLOCK_HEADER_SIZE + hdr->size > heap_end) {
        return 0; /* Inconsistent block bounds */
    }

    *out_hdr = hdr;
    return 1;
}

/* Internal allocation while holding lock */
static void *internal_alloc_locked(size_t aligned_size) {
    BlockHeader *curr = s_free_list;
    while (curr) {
        if (curr->magic != ARENA_MEM_MAGIC) {
            return NULL;
        }

        if (curr->is_free && curr->size >= aligned_size) {
            if (curr->size >= aligned_size + BLOCK_HEADER_SIZE + 16) {
                BlockHeader *new_block = (BlockHeader *)((uint8_t *)curr + BLOCK_HEADER_SIZE + aligned_size);
                new_block->magic = ARENA_MEM_MAGIC;
                new_block->size = curr->size - aligned_size - BLOCK_HEADER_SIZE;
                new_block->is_free = 1;
                new_block->pad = 0;
                new_block->next = curr->next;

                curr->size = aligned_size;
                curr->next = new_block;
            }
            curr->is_free = 0;
            s_total_allocated += curr->size;
            if (s_total_allocated > s_peak_allocated) {
                s_peak_allocated = s_total_allocated;
            }
            s_alloc_count++;
            return (void *)((uint8_t *)curr + BLOCK_HEADER_SIZE);
        }
        curr = curr->next;
    }
    return NULL;
}

/* Internal free & coalesce while holding lock */
static void internal_free_locked(BlockHeader *hdr) {
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
}

void *arenaos_malloc(size_t size) {
    if (size == 0) return NULL;
    if (size > (HEAP_SIZE - BLOCK_HEADER_SIZE)) return NULL;

    if (!s_initialized) {
        arenaos_memory_init();
    }

    size_t aligned_size = (size + 15) & ~((size_t)15);

    mem_lock();
    void *ptr = internal_alloc_locked(aligned_size);
    mem_unlock();
    return ptr;
}

void arenaos_free(void *ptr) {
    if (!ptr) return;

    mem_lock();
    BlockHeader *hdr = NULL;
    if (!is_valid_pointer(ptr, &hdr) || hdr->is_free) {
        s_bad_frees++;
        mem_unlock();
        return;
    }

    internal_free_locked(hdr);
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

    size_t aligned_size = (size + 15) & ~((size_t)15);

    mem_lock();
    BlockHeader *hdr = NULL;
    if (!is_valid_pointer(ptr, &hdr) || hdr->is_free) {
        s_bad_frees++;
        mem_unlock();
        return NULL; /* Untrusted pointer or double free: refuse safely */
    }

    size_t old_size = hdr->size;
    if (old_size >= aligned_size) {
        /* Already large enough: optionally split if significant excess */
        if (old_size >= aligned_size + BLOCK_HEADER_SIZE + 16) {
            BlockHeader *excess = (BlockHeader *)((uint8_t *)hdr + BLOCK_HEADER_SIZE + aligned_size);
            excess->magic = ARENA_MEM_MAGIC;
            excess->size = old_size - aligned_size - BLOCK_HEADER_SIZE;
            excess->is_free = 1;
            excess->pad = 0;
            excess->next = hdr->next;

            hdr->size = aligned_size;
            hdr->next = excess;
            s_total_allocated -= excess->size + BLOCK_HEADER_SIZE;

            /* Coalesce excess with next block if free */
            if (excess->next && excess->next->is_free) {
                excess->size += BLOCK_HEADER_SIZE + excess->next->size;
                excess->next = excess->next->next;
            }
        }
        mem_unlock();
        return ptr;
    }

    /* Try growing in place into immediately following free block */
    if (hdr->next && hdr->next->is_free &&
        (old_size + BLOCK_HEADER_SIZE + hdr->next->size >= aligned_size)) {
        size_t combined = old_size + BLOCK_HEADER_SIZE + hdr->next->size;
        BlockHeader *following = hdr->next->next;

        if (combined >= aligned_size + BLOCK_HEADER_SIZE + 16) {
            BlockHeader *excess = (BlockHeader *)((uint8_t *)hdr + BLOCK_HEADER_SIZE + aligned_size);
            excess->magic = ARENA_MEM_MAGIC;
            excess->size = combined - aligned_size - BLOCK_HEADER_SIZE;
            excess->is_free = 1;
            excess->pad = 0;
            excess->next = following;

            hdr->size = aligned_size;
            hdr->next = excess;
            s_total_allocated += (aligned_size - old_size);
        } else {
            hdr->size = combined;
            hdr->next = following;
            s_total_allocated += (combined - old_size);
        }
        if (s_total_allocated > s_peak_allocated) {
            s_peak_allocated = s_total_allocated;
        }
        mem_unlock();
        return ptr;
    }

    /* Allocate larger replacement block under lock */
    void *new_p = internal_alloc_locked(aligned_size);
    if (!new_p) {
        /* Heap exhausted: preserve existing allocation intact */
        mem_unlock();
        return NULL;
    }

    /* Copy old payload to new buffer */
    size_t copy_len = old_size < size ? old_size : size;
    uint8_t *dst = (uint8_t *)new_p;
    const uint8_t *src = (const uint8_t *)ptr;
    for (size_t i = 0; i < copy_len; i++) {
        dst[i] = src[i];
    }

    /* Free old block under lock */
    internal_free_locked(hdr);

    mem_unlock();
    return new_p;
}

uint64_t arenaos_mem_bad_frees(void) {
    return s_bad_frees;
}

size_t arenaos_mem_total_allocated(void) {
    return s_total_allocated;
}

size_t arenaos_mem_peak_allocated(void) {
    return s_peak_allocated;
}

#ifndef ARENAOS_HOST_TEST
/* Aliases for freestanding C runtime */
void *malloc(size_t size) __attribute__((weak, alias("arenaos_malloc")));
void free(void *ptr) __attribute__((weak, alias("arenaos_free")));
void *calloc(size_t nmemb, size_t size) __attribute__((weak, alias("arenaos_calloc")));
void *realloc(void *ptr, size_t size) __attribute__((weak, alias("arenaos_realloc")));

/* String/memory routines for freestanding environment */
__attribute__((weak)) void *memcpy(void *dest, const void *src, size_t n) {
    uint8_t *d = (uint8_t *)dest;
    const uint8_t *s = (const uint8_t *)src;
    for (size_t i = 0; i < n; i++) d[i] = s[i];
    return dest;
}

__attribute__((weak)) void *memset(void *s, int c, size_t n) {
    uint8_t *p = (uint8_t *)s;
    for (size_t i = 0; i < n; i++) p[i] = (uint8_t)c;
    return s;
}

__attribute__((weak)) void *memmove(void *dest, const void *src, size_t n) {
    uint8_t *d = (uint8_t *)dest;
    const uint8_t *s = (const uint8_t *)src;
    if (d < s) {
        for (size_t i = 0; i < n; i++) d[i] = s[i];
    } else if (d > s) {
        for (size_t i = n; i > 0; i--) d[i - 1] = s[i - 1];
    }
    return dest;
}

__attribute__((weak)) size_t strlen(const char *s) {
    size_t len = 0;
    while (s[len]) len++;
    return len;
}

__attribute__((weak)) int memcmp(const void *s1, const void *s2, size_t n) {
    const uint8_t *p1 = (const uint8_t *)s1;
    const uint8_t *p2 = (const uint8_t *)s2;
    for (size_t i = 0; i < n; i++) {
        if (p1[i] != p2[i]) {
            return (int)p1[i] - (int)p2[i];
        }
    }
    return 0;
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
#endif /* !ARENAOS_HOST_TEST */
