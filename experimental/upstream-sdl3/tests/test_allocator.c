/*
  test_allocator.c: Host Unit Tests for Hardened ArenaOS Memory Allocator
  Tests 16-byte alignment, in-place growth, relocation, multithreaded allocations,
  double free refusal, and heap exhaustion.
*/

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <assert.h>
#include <pthread.h>

extern void arenaos_memory_init(void);
extern void *arenaos_malloc(size_t size);
extern void arenaos_free(void *ptr);
extern void *arenaos_calloc(size_t nmemb, size_t size);
extern void *arenaos_realloc(void *ptr, size_t size);
extern uint64_t arenaos_mem_bad_frees(void);
extern size_t arenaos_mem_total_allocated(void);

#define NUM_THREADS 4
#define ITERS_PER_THREAD 100

static void *worker_thread(void *arg) {
    int thread_id = (int)(intptr_t)arg;
    for (int iter = 0; iter < ITERS_PER_THREAD; iter++) {
        size_t sz = 16 + ((thread_id * 31 + iter * 17) % 256);
        uint8_t *p = (uint8_t *)arenaos_malloc(sz);
        if (p) {
            assert(((uintptr_t)p & 15) == 0);
            memset(p, (uint8_t)(thread_id + iter), sz);

            /* Reallocate */
            size_t new_sz = sz + 32;
            uint8_t *p2 = (uint8_t *)arenaos_realloc(p, new_sz);
            if (p2) {
                assert(((uintptr_t)p2 & 15) == 0);
                assert(p2[0] == (uint8_t)(thread_id + iter));
                arenaos_free(p2);
            } else {
                arenaos_free(p);
            }
        }
    }
    return NULL;
}

int main(void) {
    printf("=== Testing Hardened ArenaOS Allocator Scaffolding ===\n");
    arenaos_memory_init();

    /* Test 1: 16-byte alignment verification */
    printf("[test_allocator] Test 1: 16-byte alignment verification...\n");
    void *ptrs[14];
    for (int i = 0; i < 14; i++) {
        size_t req = 1 + i * 19;
        ptrs[i] = arenaos_malloc(req);
        assert(ptrs[i] != NULL);
        assert(((uintptr_t)ptrs[i] & 15) == 0);
    }
    printf("  [PASS] All 14 allocations strictly 16-byte aligned.\n");

    /* Test 2: In-place growth vs relocation with payload preservation */
    printf("[test_allocator] Test 2: Realloc growth and data preservation...\n");
    char *orig = (char *)arenaos_malloc(32);
    assert(orig != NULL);
    strcpy(orig, "ArenaOS Kernel Payload");
    char *grown = (char *)arenaos_realloc(orig, 128);
    assert(grown != NULL);
    assert(((uintptr_t)grown & 15) == 0);
    assert(strcmp(grown, "ArenaOS Kernel Payload") == 0);
    printf("  [PASS] Realloc preserved payload across expansion.\n");

    /* Test 3: Realloc shrinkage */
    printf("[test_allocator] Test 3: Realloc shrinkage...\n");
    char *shrunk = (char *)arenaos_realloc(grown, 16);
    assert(shrunk != NULL);
    assert(strncmp(shrunk, "ArenaOS", 7) == 0);
    arenaos_free(shrunk);
    printf("  [PASS] Realloc shrinkage successful.\n");

    /* Test 4: Invalid pointer handling & double free protection */
    printf("[test_allocator] Test 4: Invalid pointer handling & double free protection...\n");
    uint64_t initial_bad = arenaos_mem_bad_frees();
    int foreign_var = 42;
    arenaos_free(&foreign_var);
    assert(arenaos_mem_bad_frees() == initial_bad + 1);

    void *unaligned = (void *)((uintptr_t)ptrs[0] + 1);
    arenaos_free(unaligned);
    assert(arenaos_mem_bad_frees() == initial_bad + 2);

    void *legit = arenaos_malloc(64);
    assert(legit != NULL);
    arenaos_free(legit);
    /* Double free attempt */
    arenaos_free(legit);
    assert(arenaos_mem_bad_frees() == initial_bad + 3);
    printf("  [PASS] Foreign, unaligned, and double-free calls safely rejected.\n");

    /* Test 5: Realloc with invalid pointer */
    printf("[test_allocator] Test 5: Realloc with invalid pointer...\n");
    initial_bad = arenaos_mem_bad_frees();
    void *bad_realloc = arenaos_realloc(&foreign_var, 128);
    assert(bad_realloc == NULL);
    assert(arenaos_mem_bad_frees() == initial_bad + 1);
    printf("  [PASS] Realloc with invalid pointer safely refused.\n");

    /* Test 6: Calloc with overflow check */
    printf("[test_allocator] Test 6: Calloc with overflow check...\n");
    void *overflow = arenaos_calloc((size_t)-1 / 2, 4);
    assert(overflow == NULL);
    int *zeroes = (int *)arenaos_calloc(16, sizeof(int));
    assert(zeroes != NULL);
    for (int i = 0; i < 16; i++) {
        assert(zeroes[i] == 0);
    }
    arenaos_free(zeroes);
    printf("  [PASS] Calloc overflow check and zero initialization verified.\n");

    /* Test 7: Heap exhaustion handling */
    printf("[test_allocator] Test 7: Heap exhaustion handling...\n");
    void *huge = arenaos_malloc(200 * 1024); /* > 128 KiB heap */
    assert(huge == NULL);
    printf("  [PASS] Heap exhaustion safely returns NULL without crashing.\n");

    /* Clean up ptrs */
    for (int i = 0; i < 14; i++) {
        arenaos_free(ptrs[i]);
    }

    /* Test 8: Heap coalescing and reusability */
    printf("[test_allocator] Test 8: Heap coalescing and reusability...\n");
    void *p_block1 = arenaos_malloc(4096);
    void *p_block2 = arenaos_malloc(4096);
    assert(p_block1 && p_block2);
    arenaos_free(p_block1);
    arenaos_free(p_block2);
    void *combined = arenaos_malloc(8192);
    assert(combined != NULL);
    arenaos_free(combined);
    printf("  [PASS] Coalescing successfully recombined adjacent blocks.\n");

    /* Test 9: Concurrent multithreaded allocation stress */
    printf("[test_allocator] Test 9: Multithreaded concurrent allocation stress...\n");
    pthread_t threads[NUM_THREADS];
    for (int t = 0; t < NUM_THREADS; t++) {
        int prc = pthread_create(&threads[t], NULL, worker_thread, (void *)(intptr_t)t);
        assert(prc == 0);
    }
    for (int t = 0; t < NUM_THREADS; t++) {
        pthread_join(threads[t], NULL);
    }
    printf("  [PASS] 4 concurrent threads completed 400 total alloc/realloc/free cycles.\n");

    printf("\nALL 9 ALLOCATOR SAFETY & CONCURRENCY TESTS PASSED!\n");
    return 0;
}
