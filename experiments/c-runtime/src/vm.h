/* Internal VM and scheduling backend (C1). One implementation for the guest
 * (SYS_VM_* and SYS_THREAD_YIELD) and one for the host test build (mmap and
 * sched_yield), selected by ARENA_HOSTED. */
#ifndef ARENA_VM_H
#define ARENA_VM_H

#include <stdint.h>

/* Reserve `pages` of process-owned virtual address space (lazy, uncommitted).
 * On success stores the region capability slot and page-aligned base.
 * Returns 0 or a negative status. */
int arena_vm_reserve(uint32_t pages, uint64_t *slot_out, uintptr_t *base_out);

/* Make `pages` pages starting at `offset_pages` read-write. 0 or negative. */
int arena_vm_commit(uint64_t slot, uint32_t offset_pages, uint32_t pages);

/* Release an exact reservation returned by arena_vm_reserve. 0 or negative. */
int arena_vm_release(uint64_t slot, uintptr_t base, uint32_t pages);

/* Yield the calling thread while a runtime lock is contended. */
void arena_backoff(void);

#endif /* ARENA_VM_H */
