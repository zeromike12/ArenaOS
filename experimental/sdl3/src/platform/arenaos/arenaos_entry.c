#include "arenaos_syscalls.h"

#define STACK_SIZE (64 * 1024)

static uint8_t s_stack[STACK_SIZE] __attribute__((aligned(4096)));
static uint8_t s_startup_snapshot[4096] __attribute__((aligned(4096)));

extern void arenaos_memory_init(void);
extern int main(int argc, char *argv[]);
extern void *memcpy(void *dest, const void *src, size_t n);

static char *s_argv[] = {"sdl3-app", NULL};

void arenaos_entry(void) {
    /* Step 1: Map and consume StartupView from Slot 0 */
    int64_t startup_va = arenaos_syscall2(SYS_SHARED_MAP, 0, 0);
    if (startup_va > 0) {
        /* Copy 4096 bytes of startup record into private static BSS */
        memcpy(s_startup_snapshot, (const void *)startup_va, 4096);

        /* Unmap and destroy Slot 0 to release transport capability */
        arenaos_syscall6(SYS_SHARED_UNMAP, (uint64_t)startup_va, 0, 0, 0, 0, 0);
        arenaos_syscall1(SYS_CAP_DESTROY, 0);
    }

    /* Step 2: Initialize memory heap */
    arenaos_memory_init();

    /* Step 3: Run application main */
    int rc = main(1, s_argv);

    /* Step 4: Terminate cleanly */
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
