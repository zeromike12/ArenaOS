/*
  ArenaOS Freestanding C Runtime Glue (Milestone G2 Temporary Scaffolding)
  Notice: This file provides minimal startup and capability validation scaffolding
  strictly scoped to Milestone G2 graphics experimentation pending Arena 2's general C runtime.
*/

#include "arenaos_syscalls.h"

#define STACK_SIZE (64 * 1024)

/* 64 KiB 4096-aligned stack in .bss */
static uint8_t s_stack[STACK_SIZE] __attribute__((aligned(4096)));
static uint8_t s_startup_snapshot[4096] __attribute__((aligned(4096)));

extern void arenaos_memory_init(void);
extern int main(int argc, char *argv[]);
extern void *memcpy(void *dest, const void *src, size_t n);

static char *s_argv[] = {"sdl3-app", NULL};

void arenaos_entry(void) {
    /*
     * Step 1: Fail-closed validation of Slot 0 (StartupView ARST capability).
     * Must be a valid SharedRegion (kind 7) with at least READ rights.
     */
    uint64_t desc0[3] = {0};
    int64_t st = arenaos_syscall2(SYS_CAP_DESCRIBE, 0, (uint64_t)desc0);
    if (st != 0 || desc0[0] != 7 || (desc0[2] & 1) == 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 0 is not a readable SharedRegion\n", 62);
        arenaos_exit(1);
    }

    /* Map Slot 0 */
    int64_t startup_va = arenaos_syscall2(SYS_SHARED_MAP, 0, 0);
    if (startup_va <= 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Failed to map Slot 0 SharedRegion\n", 57);
        arenaos_exit(1);
    }

    /* Validate ARST v2 Header */
    const uint32_t *header_u32 = (const uint32_t *)startup_va;
    const uint16_t *header_u16 = (const uint16_t *)startup_va;
    uint32_t magic = header_u32[0];
    uint16_t version = header_u16[2];

    if (magic != 0x54535241 || version != 2) {
        arenaos_debug_write("[arenaos_entry] FATAL: Malformed ARST header: bad magic or version != 2\n", 72);
        arenaos_syscall6(SYS_SHARED_UNMAP, (uint64_t)startup_va, 0, 0, 0, 0, 0);
        arenaos_syscall1(SYS_CAP_DESTROY, 0);
        arenaos_exit(1);
    }

    /* Copy 4096 bytes of startup record into private static BSS */
    memcpy(s_startup_snapshot, (const void *)startup_va, 4096);

    /* Unmap and destroy Slot 0 transport capability */
    arenaos_syscall6(SYS_SHARED_UNMAP, (uint64_t)startup_va, 0, 0, 0, 0, 0);
    arenaos_syscall1(SYS_CAP_DESTROY, 0);

    /*
     * Step 2: Fail-closed validation of standard application capabilities.
     * Slot 1: Desktop Service BadgedEndpoint (kind 12, WRITE rights)
     * Slot 2: Window Surface SharedRegion (kind 7, READ | WRITE rights)
     * Slot 3: Clock Device (kind 3)
     */
    uint64_t desc1[3] = {0};
    if (arenaos_syscall2(SYS_CAP_DESCRIBE, SERVICE_ENDPOINT, (uint64_t)desc1) != 0 ||
        desc1[0] != 12 || (desc1[2] & 2) == 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 1 is not a valid desktop endpoint\n", 63);
        arenaos_exit(1);
    }

    uint64_t desc2[3] = {0};
    if (arenaos_syscall2(SYS_CAP_DESCRIBE, SURFACE_SLOT, (uint64_t)desc2) != 0 ||
        desc2[0] != 7 || (desc2[2] & 3) != 3) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 2 is not a valid read/write surface\n", 65);
        arenaos_exit(1);
    }

    uint64_t desc3[3] = {0};
    if (arenaos_syscall2(SYS_CAP_DESCRIBE, CLOCK_SLOT, (uint64_t)desc3) != 0 ||
        desc3[0] != 3) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 3 is not a valid clock device\n", 58);
        arenaos_exit(1);
    }

    /* Step 3: Initialize temporary memory heap scaffolding */
    arenaos_memory_init();

    /* Step 4: Run application main */
    int rc = main(1, s_argv);

    /* Step 5: Terminate cleanly */
    arenaos_exit(rc);
}

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "lea %0, %%rsp\n"
        "and $-16, %%rsp\n"
        "call arenaos_entry\n"
        "ud2\n"
        :
        : "m"(s_stack[STACK_SIZE - 16])
        : "memory"
    );
}
