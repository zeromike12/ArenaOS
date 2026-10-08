/*
  ArenaOS Freestanding Memory Allocator (Milestone G2 Temporary Scaffolding)
  Notice: This file provides minimal heap allocation scaffolding strictly scoped
  to Milestone G2 graphics experimentation pending Arena 2's general C runtime.
  It is not a general C runtime allocator and is not exported outside experimental/sdl3.
*/

#include "arenaos_syscalls.h"

#define HEAP_SIZE (128 * 1024) /* 128 KiB static heap */


static uint8_t s_heap[HEAP_SIZE] __attribute__((aligned(16)));

typedef struct BlockHeader {
    size_t size;               /* Payload size in bytes */
    int is_free;               /* 1 if free, 0 if allocated */
    struct BlockHeader *next;  /* Next block in heap list */
} BlockHeader;

#define BLOCK_HEADER_SIZE (sizeof(BlockHeader))

static BlockHeader *s_free_list = NULL;
static size_t s_total_allocated = 0;
static size_t s_peak_allocated = 0;
static size_t s_alloc_count = 0;

void arenaos_memory_init(void) {
    s_free_list = (BlockHeader *)s_heap;
    s_free_list->size = HEAP_SIZE - BLOCK_HEADER_SIZE;
    s_free_list->is_free = 1;
    s_free_list->next = NULL;
    s_total_allocated = 0;
    s_peak_allocated = 0;
    s_alloc_count = 0;
}

void *malloc(size_t size) {
    if (size == 0) return NULL;
    if (!s_free_list) {
        arenaos_memory_init();
    }

    /* 16-byte align size */
    size = (size + 15) & ~((size_t)15);

    BlockHeader *curr = s_free_list;
    while (curr) {
        if (curr->is_free && curr->size >= size) {
            /* Check if we can split this block */
            if (curr->size >= size + BLOCK_HEADER_SIZE + 32) {
                BlockHeader *new_block = (BlockHeader *)((uint8_t *)curr + BLOCK_HEADER_SIZE + size);
                new_block->size = curr->size - size - BLOCK_HEADER_SIZE;
                new_block->is_free = 1;
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
            return (void *)((uint8_t *)curr + BLOCK_HEADER_SIZE);
        }
        curr = curr->next;
    }

    /* Out of memory */
    const char oom_msg[] = "[arenaos_mem] FATAL: Heap exhausted (512 KiB limit reached)!\n";
    arenaos_debug_write(oom_msg, sizeof(oom_msg) - 1);
    return NULL;
}

void free(void *ptr) {
    if (!ptr) return;
    if ((uint8_t *)ptr < s_heap || (uint8_t *)ptr >= s_heap + HEAP_SIZE) {
        return;
    }

    BlockHeader *hdr = (BlockHeader *)((uint8_t *)ptr - BLOCK_HEADER_SIZE);
    if (hdr->is_free) {
        return; /* Double free protection */
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
}

void *calloc(size_t nmemb, size_t size) {
    size_t total = nmemb * size;
    if (nmemb != 0 && total / nmemb != size) {
        return NULL; /* Overflow */
    }
    void *p = malloc(total);
    if (p) {
        uint8_t *b = (uint8_t *)p;
        for (size_t i = 0; i < total; i++) {
            b[i] = 0;
        }
    }
    return p;
}

void *realloc(void *ptr, size_t size) {
    if (!ptr) return malloc(size);
    if (size == 0) {
        free(ptr);
        return NULL;
    }

    BlockHeader *hdr = (BlockHeader *)((uint8_t *)ptr - BLOCK_HEADER_SIZE);
    size = (size + 15) & ~((size_t)15);

    if (hdr->size >= size) {
        return ptr;
    }

    void *new_p = malloc(size);
    if (new_p) {
        uint8_t *src = (uint8_t *)ptr;
        uint8_t *dst = (uint8_t *)new_p;
        for (size_t i = 0; i < hdr->size; i++) {
            dst[i] = src[i];
        }
        free(ptr);
    }
    return new_p;
}

size_t arenaos_mem_peak(void) {
    return s_peak_allocated;
}

size_t arenaos_mem_current(void) {
    return s_total_allocated;
}

/* Freestanding string and memory operations */
void *memcpy(void *dest, const void *src, size_t n) {
    uint8_t *d = (uint8_t *)dest;
    const uint8_t *s = (const uint8_t *)src;
    for (size_t i = 0; i < n; i++) {
        d[i] = s[i];
    }
    return dest;
}

void *memset(void *s, int c, size_t n) {
    uint8_t *p = (uint8_t *)s;
    for (size_t i = 0; i < n; i++) {
        p[i] = (uint8_t)c;
    }
    return s;
}

void *memmove(void *dest, const void *src, size_t n) {
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

int memcmp(const void *s1, const void *s2, size_t n) {
    const uint8_t *p1 = (const uint8_t *)s1;
    const uint8_t *p2 = (const uint8_t *)s2;
    for (size_t i = 0; i < n; i++) {
        if (p1[i] != p2[i]) {
            return p1[i] - p2[i];
        }
    }
    return 0;
}

size_t strlen(const char *s) {
    size_t len = 0;
    while (s && s[len]) {
        len++;
    }
    return len;
}

int strcmp(const char *s1, const char *s2) {
    while (*s1 && (*s1 == *s2)) {
        s1++;
        s2++;
    }
    return *(const unsigned char *)s1 - *(const unsigned char *)s2;
}

int strncmp(const char *s1, const char *s2, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (s1[i] != s2[i] || s1[i] == '\0') {
            return (unsigned char)s1[i] - (unsigned char)s2[i];
        }
    }
    return 0;
}
