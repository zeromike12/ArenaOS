/*
  Host Unit Test for Hardened Freestanding Allocator Scaffolding (Milestone G2.2)
*/

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <assert.h>

/* Forward declarations of the allocator under test */
void arenaos_memory_init(void);
void *arenaos_malloc(size_t size);
void arenaos_free(void *ptr);
void *arenaos_calloc(size_t nmemb, size_t size);
void *arenaos_realloc(void *ptr, size_t size);
size_t arenaos_mem_peak(void);
size_t arenaos_mem_current(void);
size_t arenaos_mem_bad_frees(void);

int main(void) {
    printf("=== Testing Hardened ArenaOS Allocator Scaffolding ===\n");
    arenaos_memory_init();

    /* Test 1: 16-byte alignment across varied allocation sizes */
    printf("[test_allocator] Test 1: 16-byte alignment verification...\n");
    size_t test_sizes[] = {1, 3, 7, 15, 16, 17, 31, 32, 48, 64, 128, 256, 1024, 4096};
    void *ptrs[14];
    for (int i = 0; i < 14; i++) {
        ptrs[i] = arenaos_malloc(test_sizes[i]);
        assert(ptrs[i] != NULL);
        uintptr_t addr = (uintptr_t)ptrs[i];
        if ((addr % 16) != 0) {
            fprintf(stderr, "FAIL: Pointer %p for size %zu is not 16-byte aligned (offset %lu)!\n",
                    ptrs[i], test_sizes[i], addr % 16);
            return 1;
        }
        /* Write test pattern */
        uint8_t *b = (uint8_t *)ptrs[i];
        for (size_t k = 0; k < test_sizes[i]; k++) {
            b[k] = (uint8_t)(k & 0xFF);
        }
    }
    printf("  [PASS] All 14 allocations strictly 16-byte aligned.\n");

    /* Test 2: Realloc growth and payload preservation */
    printf("[test_allocator] Test 2: Realloc growth and data preservation...\n");
    char *orig = (char *)arenaos_malloc(32);
    assert(orig != NULL);
    const char msg[] = "Hello, ArenaOS Upstream SDL3!";
    for (size_t i = 0; i < sizeof(msg); i++) orig[i] = msg[i];

    char *grown = (char *)arenaos_realloc(orig, 256);
    assert(grown != NULL);
    assert(((uintptr_t)grown % 16) == 0);
    for (size_t i = 0; i < sizeof(msg); i++) {
        assert(grown[i] == msg[i]);
    }
    printf("  [PASS] Realloc preserved payload across expansion.\n");

    /* Test 3: Realloc shrinkage */
    printf("[test_allocator] Test 3: Realloc shrinkage...\n");
    char *shrunk = (char *)arenaos_realloc(grown, 16);
    assert(shrunk != NULL);
    assert(((uintptr_t)shrunk % 16) == 0);
    for (size_t i = 0; i < 15; i++) {
        assert(shrunk[i] == msg[i]);
    }
    arenaos_free(shrunk);
    printf("  [PASS] Realloc shrinkage successful.\n");

    /* Free all initial allocations */
    for (int i = 0; i < 14; i++) {
        arenaos_free(ptrs[i]);
    }

    /* Test 4: Invalid pointer rejection & double free protection */
    printf("[test_allocator] Test 4: Invalid pointer handling & double free protection...\n");
    size_t initial_bad = arenaos_mem_bad_frees();

    /* 4a: Foreign stack pointer */
    int stack_var = 42;
    arenaos_free(&stack_var);
    assert(arenaos_mem_bad_frees() == initial_bad + 1);

    /* 4b: Arbitrary foreign address */
    void *foreign_addr = (void *)0x1234567800ULL;
    arenaos_free(foreign_addr);
    assert(arenaos_mem_bad_frees() == initial_bad + 2);

    /* 4c: Unaligned pointer inside our heap */
    void *valid_p = arenaos_malloc(64);
    assert(valid_p != NULL);
    void *unaligned_p = (void *)((uintptr_t)valid_p + 1);
    arenaos_free(unaligned_p);
    assert(arenaos_mem_bad_frees() == initial_bad + 3);

    /* 4d: Double free protection */
    arenaos_free(valid_p);
    size_t bad_before_double = arenaos_mem_bad_frees();
    arenaos_free(valid_p); /* Second free: must be detected and rejected */
    assert(arenaos_mem_bad_frees() == bad_before_double + 1);
    printf("  [PASS] Foreign, unaligned, and double-free calls safely rejected.\n");

    /* Test 5: Realloc with invalid pointer */
    printf("[test_allocator] Test 5: Realloc with invalid pointer...\n");
    void *bad_realloc = arenaos_realloc(&stack_var, 128);
    assert(bad_realloc == NULL);
    printf("  [PASS] Realloc with invalid pointer safely refused.\n");

    /* Test 6: Calloc with overflow check */
    printf("[test_allocator] Test 6: Calloc with overflow check...\n");
    void *overflow_p = arenaos_calloc((size_t)-1 / 2, 4);
    assert(overflow_p == NULL);
    void *zero_p = arenaos_calloc(8, 16);
    assert(zero_p != NULL);
    uint8_t *zb = (uint8_t *)zero_p;
    for (int i = 0; i < 128; i++) {
        assert(zb[i] == 0);
    }
    arenaos_free(zero_p);
    printf("  [PASS] Calloc overflow check and zero initialization verified.\n");

    /* Test 7: Heap exhaustion check */
    printf("[test_allocator] Test 7: Heap exhaustion handling...\n");
    void *huge = arenaos_malloc(256 * 1024); /* 256 KiB exceeds 128 KiB heap */
    assert(huge == NULL);
    printf("  [PASS] Heap exhaustion safely returns NULL without crashing.\n");

    /* Test 8: Coalescing verification */
    printf("[test_allocator] Test 8: Heap coalescing and reusability...\n");
    void *p1 = arenaos_malloc(32 * 1024);
    void *p2 = arenaos_malloc(32 * 1024);
    assert(p1 && p2);
    arenaos_free(p1);
    arenaos_free(p2);
    /* Should now be able to allocate a single 64 KiB block */
    void *big = arenaos_malloc(64 * 1024);
    assert(big != NULL);
    arenaos_free(big);
    printf("  [PASS] Coalescing successfully recombined adjacent blocks.\n");

    printf("\nALL 8 ALLOCATOR SAFETY TESTS PASSED!\n");
    return 0;
}
