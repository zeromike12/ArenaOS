/* Internal VM backend interface (prototype). One implementation for the
 * guest (SYS_VM_RESERVE / SYS_VM_COMMIT) and one for the host test build
 * (mmap), selected by ARENA_HOSTED. */
#ifndef ARENA_VM_H
#define ARENA_VM_H

#include <stdint.h>

/* Reserve `pages` of process-owned virtual address space. On success stores
 * the kernel region capability slot and the page-aligned base address.
 * Returns 0 or a negative status. */
int arena_vm_reserve(uint32_t pages, uint64_t *slot_out, uintptr_t *base_out);

/* Make `pages` pages starting at `offset_pages` of the region read-write.
 * Returns 0 or a negative status. */
int arena_vm_commit(uint64_t slot, uint32_t offset_pages, uint32_t pages);

#endif /* ARENA_VM_H */
