/*
 * Guest C startup (C1): _start -> ARST v2 gate -> stack switch -> TLS ->
 * .init_array -> main(argc, argv) -> arena_exit.
 *
 * Order matters:
 *   - The startup gate runs first, on the kernel's initial page, using only
 *     static storage. The record's slot 0 is destroyed and the live capability
 *     table is verified before ANY runtime call creates a capability (a VM
 *     reservation mints one). A VM reservation before the gate would make the
 *     "unlisted slot must be empty" rule impossible to satisfy.
 *   - Stack: the kernel gives the initial thread one 4 KiB page. We reserve a
 *     256 KiB process VM region and move RSP into it before application code.
 *   - TLS: the ArenaOS loader consumes only PT_LOAD and ignores PT_TLS, so this
 *     file installs the block itself using the x86-64 variant-II layout (TLS
 *     block below the thread pointer, TCB self-pointer at %fs:0).
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "sysabi.h"
#include "libc_impl.h"
#include "arena/rt.h"
#include "arena/string.h"
#include "internal.h"
#include "vm.h"

/* Layout anchor: a zero-initialised TLS variable that is always linked, so
 * .tbss is never empty. GNU ld drops an empty .tbss output section while LLD
 * keeps it (padded to the TLS alignment), so without this the PT_TLS memsz
 * differs by toolchain and the __arena_tls_memsz formula in arena-user.ld
 * would be wrong for one of them. Never read or written. */
static __thread uint64_t arena_tls_layout_anchor[2] __attribute__((used, aligned(16)));

#define STACK_PAGES 64u
#define STACK_BYTES (STACK_PAGES * ARENA_PAGE_SIZE)

/* Linker-provided (experiments/c-runtime/link/arena-user.ld). Their ADDRESSES
 * are values: sizes/alignment are read with (uintptr_t)&symbol. */
extern const char __arena_tls_image[];
extern const char __arena_tls_filesz[];
extern const char __arena_tls_memsz[];
extern const char __arena_tls_align[];
extern void (*__init_array_start[])(void) __attribute__((visibility("hidden")));
extern void (*__init_array_end[])(void) __attribute__((visibility("hidden")));

static uintptr_t stack_lo;
static uintptr_t stack_hi;
static int tls_ok;

/* The one assembly stub the runtime cannot avoid: first instruction of the
 * ELF entry point. Clears the frame pointer, aligns RSP to 16 bytes (the
 * kernel's initial RSP is only 8-byte aligned), and calls the C half. */
__asm__(
    ".section .text._start,\"ax\",@progbits\n"
    ".globl _start\n"
    ".type _start,@function\n"
    "_start:\n"
    "    xor %ebp, %ebp\n"
    "    and $-16, %rsp\n"
    "    call arena_crt_start\n"
    "    ud2\n"
    ".size _start, .-_start\n"
    ".text\n"
    ".globl arena_switch_stack\n"
    ".type arena_switch_stack,@function\n"
    "arena_switch_stack:\n" /* rdi = new stack top, rsi = noreturn entry */
    "    mov %rdi, %rsp\n"
    "    xor %ebp, %ebp\n"
    "    call *%rsi\n"
    "    ud2\n"
    ".size arena_switch_stack, .-arena_switch_stack\n");

void arena_switch_stack(uintptr_t top, void (*fn)(void)) __attribute__((noreturn));

void arena_stack_bounds(uintptr_t *lo, uintptr_t *hi) {
    if (lo != NULL) {
        *lo = stack_lo;
    }
    if (hi != NULL) {
        *hi = stack_hi;
    }
}

int arena_tls_ready(void) {
    return tls_ok;
}

static int stack_init(void) {
    uint64_t slot = 0;
    uintptr_t base = 0;
    if (arena_vm_reserve(STACK_PAGES, &slot, &base) != 0) {
        return -1;
    }
    if (arena_vm_commit(slot, 0, STACK_PAGES) != 0) {
        return -1;
    }
    stack_lo = base;
    stack_hi = base + STACK_BYTES;
    return 0;
}

static int tls_init(void) {
    uintptr_t memsz = (uintptr_t)__arena_tls_memsz;
    uintptr_t filesz = (uintptr_t)__arena_tls_filesz;
    uintptr_t align = (uintptr_t)__arena_tls_align;
    if (align == 0) {
        align = 1;
    }
    /* The kernel requires a 16-byte aligned FS.base. Alignments that do not
     * divide 16 are refused rather than silently mis-placed. */
    if (align > 16 || (16 % align) != 0 || filesz > memsz) {
        return -1;
    }
    uintptr_t tls_round = (memsz + align - 1) / align * align;
    uintptr_t shift = (16 - tls_round % 16) % 16;
    uintptr_t total = shift + tls_round + 64; /* 64: slack for the TCB word */
    uint8_t *block = arena_malloc((size_t)total);
    if (block == NULL) {
        return -1;
    }
    uint8_t *image = block + shift;              /* TP - tls_round */
    uintptr_t tp = (uintptr_t)image + tls_round; /* 16-byte aligned */
    arena_memcpy(image, __arena_tls_image, (size_t)filesz);
    arena_memset(image + filesz, 0, (size_t)(memsz - filesz));
    *(uintptr_t *)tp = tp; /* TCB self-pointer: %fs:0 */
    int64_t rc = arena_syscall6(ARENA_SYS_TLS_SET, tp, 0, 0, 0, 0, 0);
    if (rc != ARENA_STATUS_OK) {
        return -1;
    }
    tls_ok = 1;
    return 0;
}

static void run_init_array(void) {
    size_t n = (size_t)(__init_array_end - __init_array_start);
    for (size_t i = 0; i < n; i++) {
        __init_array_start[i]();
    }
}

static void crt_main(void) __attribute__((noreturn));
static void crt_main(void) {
    if (tls_init() != 0) {
        arena_exit(121);
    }
    run_init_array();
    const arena_startup_t *s = arena_startup();
    int rc = s->present ? main(s->argc, s->argv) : main(0, NULL);
    arena_libc_exit(rc);
}

void arena_crt_start(void) __attribute__((noreturn));
void arena_crt_start(void) {
    /* Gate BEFORE any allocation or VM reservation (see the header comment). */
    if (arena_startup_gate() != 0) {
        arena_exit(122);
    }
    if (stack_init() != 0) {
        arena_exit(120);
    }
    arena_switch_stack(stack_hi, crt_main);
}
