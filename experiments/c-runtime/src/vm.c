/* VM and scheduling backend (C1). Guest: real kernel syscalls. Hosted: mmap,
 * sched_yield. */
#include "vm.h"

#ifdef ARENA_HOSTED

#include <stddef.h>
#include <sched.h>
#include <sys/mman.h>

/* Host backend: mappings are committed eagerly; the allocator still tracks
 * its own commit chunks, so accounting is exercised.
 *
 * Deterministic fault injection (host tests only): "fail from call N" makes
 * the Nth and every later call of that kind return a refusal. Counters reset
 * when a fail point is armed. This is how the host suite exercises VM
 * reservation and commit refusal; the guest backend has no such hook. */
static int64_t host_reserve_calls, host_commit_calls;
static int64_t host_reserve_fail_from = -1, host_commit_fail_from = -1;

void arena_vm_host_fail_reserve_from(int64_t n) {
    host_reserve_calls = 0;
    host_reserve_fail_from = n;
}

void arena_vm_host_fail_commit_from(int64_t n) {
    host_commit_calls = 0;
    host_commit_fail_from = n;
}

void arena_vm_host_reset(void) {
    host_reserve_fail_from = -1;
    host_commit_fail_from = -1;
}

int arena_vm_reserve(uint32_t pages, uint64_t *slot_out, uintptr_t *base_out) {
    int64_t k = host_reserve_calls++;
    if (host_reserve_fail_from >= 0 && k >= host_reserve_fail_from) {
        return -1;
    }
    void *p = mmap(NULL, (size_t)pages * 4096u, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) {
        return -1;
    }
    *slot_out = 1; /* host stand-in for a capability slot */
    *base_out = (uintptr_t)p;
    return 0;
}

int arena_vm_commit(uint64_t slot, uint32_t offset_pages, uint32_t pages) {
    (void)slot;
    (void)offset_pages;
    (void)pages;
    int64_t k = host_commit_calls++;
    if (host_commit_fail_from >= 0 && k >= host_commit_fail_from) {
        return -1;
    }
    return 0;
}

int arena_vm_release(uint64_t slot, uintptr_t base, uint32_t pages) {
    (void)slot;
    return munmap((void *)base, (size_t)pages * 4096u) == 0 ? 0 : -1;
}

void arena_backoff(void) {
    sched_yield();
}

#else

#include "arena/abi.h"

/* Reply words: [slot, base, pages]. The kernel writes them only on success
 * (syscall.rs sys_vm_reserve). */
int arena_vm_reserve(uint32_t pages, uint64_t *slot_out, uintptr_t *base_out) {
    uint64_t out[3] = {0, 0, 0};
    int64_t rc = arena_syscall6(ARENA_SYS_VM_RESERVE, pages, (uint64_t)(uintptr_t)out,
                                0, 0, 0, 0);
    if (rc != ARENA_STATUS_OK) {
        return (int)rc;
    }
    if (out[0] >= 128u || out[1] == 0 || (out[1] & (ARENA_PAGE_SIZE - 1u)) != 0 ||
        out[2] < pages) {
        return -1; /* malformed reply: refuse rather than trust it */
    }
    *slot_out = out[0];
    *base_out = (uintptr_t)out[1];
    return 0;
}

int arena_vm_commit(uint64_t slot, uint32_t offset_pages, uint32_t pages) {
    int64_t rc = arena_syscall6(ARENA_SYS_VM_COMMIT, slot, offset_pages, pages,
                                ARENA_VM_PROT_READ | ARENA_VM_PROT_WRITE, 0, 0);
    return rc == ARENA_STATUS_OK ? 0 : (int)rc;
}

int arena_vm_release(uint64_t slot, uintptr_t base, uint32_t pages) {
    (void)base;
    (void)pages;
    int64_t rc = arena_syscall6(ARENA_SYS_VM_RELEASE, slot, 0, 0, 0, 0, 0);
    return rc == ARENA_STATUS_OK ? 0 : (int)rc;
}

void arena_backoff(void) {
    (void)arena_syscall6(ARENA_SYS_THREAD_YIELD, 0, 0, 0, 0, 0, 0);
}

#endif
