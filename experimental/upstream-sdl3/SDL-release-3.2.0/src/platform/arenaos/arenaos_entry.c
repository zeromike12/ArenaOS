/*
  ArenaOS Freestanding C Runtime Entry & Hardened Startup Gate (Milestone G2.2)
  Implements fail-closed ARST v2 validation and live capability inventory verification
  conforming to ADR-0083, ADR-0086, and Arena 2's native C runtime contract.
*/

#include "arenaos_syscalls.h"

#define STACK_SIZE (64 * 1024)
#define STARTUP_BLOCK_BYTES 4096u
#define STARTUP_HEADER_BYTES 128u
#define STARTUP_CAPABILITY_MAX 7u
#define STARTUP_ARGUMENT_MAX 32u
#define STARTUP_ENVIRONMENT_MAX 32u
#define STARTUP_STRING_BYTES_MAX 3072u
#define STARTUP_VERSION 2u
#define STARTUP_NONE 0xFFFFu

#define CAP_KIND_IMAGE 1u
#define CAP_KIND_ENDPOINT 2u
#define CAP_KIND_NOTIFICATION 3u
#define CAP_KIND_PROCESS 4u
#define CAP_KIND_IMAGE_REGISTRAR 5u
#define CAP_KIND_BOOT_IMAGE 6u
#define CAP_KIND_SHARED_REGION 7u
#define CAP_KIND_MEMORY_POOL 8u
#define CAP_KIND_SHARED_DMA 9u
#define CAP_KIND_PROOF_TOKEN 10u
#define CAP_KIND_UNTYPED 11u
#define CAP_KIND_BADGED_ENDPOINT 12u
#define CAP_KIND_RTC 13u
#define CAP_KIND_VM_REGION 14u
#define CAP_KIND_SYNC_DOMAIN 16u

#define RIGHT_READ 1u
#define RIGHT_WRITE 2u
#define RIGHT_COPY 4u
#define RIGHT_DESTROY 8u
#define RIGHTS_MASK 0x0Fu

#define FLAG_MULTI_INSTANCE (1u << 0)
#define FLAG_BACKGROUND (1u << 1)
#define FLAG_HEADLESS (1u << 2)
#define FLAG_STANDARD_STREAMS (1u << 3)
#define FLAG_NATIVE_SYNC (1u << 4)
#define KNOWN_FLAGS (FLAG_MULTI_INSTANCE | FLAG_BACKGROUND | FLAG_HEADLESS | FLAG_STANDARD_STREAMS | FLAG_NATIVE_SYNC)

#define USER_ADDRESS_END 0x0000800000000000ULL

/* 64 KiB 4096-aligned stack in .bss */
static uint8_t s_stack[STACK_SIZE] __attribute__((aligned(4096)));
/* 4 KiB 4096-aligned snapshot buffer in .bss */
static uint8_t s_startup_snapshot[STARTUP_BLOCK_BYTES] __attribute__((aligned(4096)));

typedef struct {
    uint16_t slot;
    uint8_t role;
    uint8_t kind;
    uint32_t rights;
} CapDesc;

static CapDesc s_verified_caps[STARTUP_CAPABILITY_MAX];
static uint16_t s_cap_count = 0;
static uint32_t s_flags = 0;

/* String storage for argv and envp */
static char s_string_store[STARTUP_STRING_BYTES_MAX + STARTUP_ARGUMENT_MAX + STARTUP_ENVIRONMENT_MAX + 1];
static char *s_argv_ptrs[STARTUP_ARGUMENT_MAX + 1];
static int s_argc = 0;

extern void arenaos_memory_init(void);
extern int main(int argc, char *argv[]);
extern void *memcpy(void *dest, const void *src, size_t n);

static uint16_t rd16(const uint8_t *p, size_t at) {
    return (uint16_t)(p[at] | ((uint16_t)p[at + 1] << 8));
}

static uint32_t rd32(const uint8_t *p, size_t at) {
    return (uint32_t)p[at] | ((uint32_t)p[at + 1] << 8) | ((uint32_t)p[at + 2] << 16) | ((uint32_t)p[at + 3] << 24);
}

static uint64_t rd64(const uint8_t *p, size_t at) {
    return (uint64_t)rd32(p, at) | ((uint64_t)rd32(p, at + 4) << 32);
}

static int all_zero(const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (p[i] != 0) return 0;
    }
    return 1;
}

static int valid_app_id(const uint8_t *id) {
    size_t end = 0;
    while (end < 32 && id[end] != 0) {
        end++;
    }
    if (end < 1 || end > 31 || !all_zero(id + end, 32 - end)) {
        return 0;
    }
    if (!((id[0] >= 'a' && id[0] <= 'z') || (id[0] >= '0' && id[0] <= '9'))) {
        return 0;
    }
    for (size_t i = 0; i < end; i++) {
        uint8_t c = id[i];
        if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '-')) {
            return 0;
        }
    }
    return 1;
}

static int valid_cap_kind(uint8_t kind) {
    return (kind >= 1u && kind <= CAP_KIND_RTC) || kind == CAP_KIND_SYNC_DOMAIN;
}

/*
 * Phase 1: Ingest and Snapshot ARST v2 page from Slot 0.
 */
static int ingest_startup_page(void) {
    int64_t occ = arenaos_syscall1(SYS_CAP_OCCUPIED, 0);
    if (occ != 1) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 0 empty or invalid\n", 48);
        return -1;
    }

    uint64_t obs[3] = {0};
    if (arenaos_syscall2(SYS_CAP_DESCRIBE, 0, (uint64_t)obs) != 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Failed to describe Slot 0\n", 49);
        return -1;
    }

    int discardable = (obs[0] == CAP_KIND_SHARED_REGION) && ((obs[2] & RIGHT_DESTROY) != 0);
    if (obs[0] != CAP_KIND_SHARED_REGION || obs[2] != (RIGHT_READ | RIGHT_DESTROY)) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 0 is not SharedRegion with READ|DESTROY\n", 68);
        if (discardable) {
            arenaos_syscall1(SYS_CAP_DESTROY, 0);
        }
        return -1;
    }

    int64_t pages = arenaos_syscall1(SYS_SHARED_PAGES, 0);
    if (pages != 1) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 0 is not 1 page\n", 45);
        arenaos_syscall1(SYS_CAP_DESTROY, 0);
        return -1;
    }

    int64_t va = arenaos_syscall2(SYS_SHARED_MAP, 0, 0);
    if (va <= 0 || ((uint64_t)va & (STARTUP_BLOCK_BYTES - 1u)) != 0 ||
        (uint64_t)va + STARTUP_BLOCK_BYTES > USER_ADDRESS_END) {
        arenaos_debug_write("[arenaos_entry] FATAL: Failed to map Slot 0\n", 45);
        if (va > 0) {
            arenaos_syscall6(SYS_SHARED_UNMAP, (uint64_t)va, 0, 0, 0, 0, 0);
        }
        arenaos_syscall1(SYS_CAP_DESTROY, 0);
        return -1;
    }

    /* Copy into private BSS buffer */
    const uint8_t *src = (const uint8_t *)(uintptr_t)va;
    for (size_t i = 0; i < STARTUP_BLOCK_BYTES; i++) {
        s_startup_snapshot[i] = src[i];
    }

    /* Unmap and destroy Slot 0 transport capability */
    int64_t unmap_rc = arenaos_syscall6(SYS_SHARED_UNMAP, (uint64_t)va, 0, 0, 0, 0, 0);
    int64_t destroy_rc = arenaos_syscall1(SYS_CAP_DESTROY, 0);
    if (unmap_rc != 0 || destroy_rc != 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Failed to unmap/destroy Slot 0\n", 54);
        return -1;
    }

    return 0;
}

/*
 * Phase 2: Canonical ARST v2 Layout Validation.
 */
static int parse_and_validate_record(void) {
    const uint8_t *p = s_startup_snapshot;

    /* Magic and version check */
    if (p[0] != 'A' || p[1] != 'R' || p[2] != 'S' || p[3] != 'T') {
        arenaos_debug_write("[arenaos_entry] FATAL: Bad ARST magic\n", 39);
        return -1;
    }
    if (rd16(p, 4) != STARTUP_VERSION || rd16(p, 6) != STARTUP_HEADER_BYTES) {
        arenaos_debug_write("[arenaos_entry] FATAL: Bad ARST version or header length\n", 57);
        return -1;
    }

    /* Padding bytes must be zero */
    if (p[50] || p[51] || p[66] || p[67] || p[100] || p[101] || p[102] || p[103]) {
        arenaos_debug_write("[arenaos_entry] FATAL: Non-zero padding in ARST header\n", 55);
        return -1;
    }

    uint32_t total = rd32(p, 8);
    s_flags = rd32(p, 12);
    uint16_t argc = rd16(p, 60);
    uint16_t envc = rd16(p, 62);
    s_cap_count = rd16(p, 64);
    uint32_t args_off = rd32(p, 68);
    uint32_t env_off = rd32(p, 72);
    uint32_t caps_off = rd32(p, 76);
    uint32_t strings_off = rd32(p, 80);
    uint32_t strings_len = rd32(p, 84);
    uint16_t instance_slot = rd16(p, 48);
    uint64_t generation = rd64(p, 52);

    if (total < STARTUP_HEADER_BYTES || total > STARTUP_BLOCK_BYTES ||
        argc == 0 || argc > STARTUP_ARGUMENT_MAX ||
        envc > STARTUP_ENVIRONMENT_MAX ||
        s_cap_count > STARTUP_CAPABILITY_MAX ||
        strings_len > STARTUP_STRING_BYTES_MAX ||
        instance_slot >= 32u || generation == 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: ARST fields outside canonical bounds\n", 60);
        return -1;
    }

    if (s_flags & ~KNOWN_FLAGS) {
        arenaos_debug_write("[arenaos_entry] FATAL: Unknown ARST flags\n", 42);
        return -1;
    }

    if (!valid_app_id(p + 16)) {
        arenaos_debug_write("[arenaos_entry] FATAL: Malformed app ID in ARST\n", 48);
        return -1;
    }

    uint32_t expect_args = STARTUP_HEADER_BYTES + (uint32_t)s_cap_count * 16u;
    uint32_t expect_env = expect_args + (uint32_t)argc * 8u;
    uint32_t expect_strings = expect_env + (uint32_t)envc * 8u;
    if (caps_off != STARTUP_HEADER_BYTES || args_off != expect_args ||
        env_off != expect_env || strings_off != expect_strings ||
        (uint64_t)strings_off + strings_len != total) {
        arenaos_debug_write("[arenaos_entry] FATAL: Non-canonical ARST offsets\n", 50);
        return -1;
    }

    if (!all_zero(p + total, STARTUP_BLOCK_BYTES - total)) {
        arenaos_debug_write("[arenaos_entry] FATAL: Unused ARST bytes not zeroed\n", 52);
        return -1;
    }

    /* Validate capability descriptors */
    for (unsigned i = 0; i < s_cap_count; i++) {
        size_t at = STARTUP_HEADER_BYTES + (size_t)i * 16u;
        uint16_t slot = rd16(p, at);
        uint8_t role = p[at + 2];
        uint8_t kind = p[at + 3];
        uint32_t rights = rd32(p, at + 4);

        if (slot != i + 1u || !valid_cap_kind(kind) || rights == 0 || (rights & ~RIGHTS_MASK)) {
            arenaos_debug_write("[arenaos_entry] FATAL: Invalid descriptor in ARST\n", 50);
            return -1;
        }
        if (!all_zero(p + at + 8, 8)) {
            arenaos_debug_write("[arenaos_entry] FATAL: Descriptor padding not zeroed\n", 53);
            return -1;
        }

        s_verified_caps[i].slot = slot;
        s_verified_caps[i].role = role;
        s_verified_caps[i].kind = kind;
        s_verified_caps[i].rights = rights;
    }

    /* Copy arguments into private store */
    size_t pos = 0;
    for (uint32_t i = 0; i < argc; i++) {
        size_t at = args_off + (size_t)i * 8u;
        uint32_t off = rd32(p, at);
        uint32_t len = rd32(p, at + 4);
        s_argv_ptrs[i] = &s_string_store[pos];
        for (uint32_t k = 0; k < len; k++) {
            s_string_store[pos + k] = (char)p[strings_off + off + k];
        }
        s_string_store[pos + len] = '\0';
        pos += len + 1u;
    }
    s_argv_ptrs[argc] = NULL;
    s_argc = (int)argc;

    return 0;
}

/*
 * Phase 3: Exact Live Capability Inventory Check.
 */
static int verify_live_inventory(void) {
    for (uint32_t slot = 0; slot < 128u; slot++) {
        int listed = (slot > 0 && slot <= s_cap_count);
        int64_t occ = arenaos_syscall1(SYS_CAP_OCCUPIED, slot);
        if (occ != 0 && occ != 1) {
            arenaos_debug_write("[arenaos_entry] FATAL: Capability occupied query failed\n", 56);
            return -1;
        }
        if (listed != (int)occ) {
            /* Listed capability missing, or unlisted capability unexpectedly granted */
            arenaos_debug_write("[arenaos_entry] FATAL: Capability inventory mismatch against ARST\n", 66);
            return -1;
        }
        if (listed) {
            uint64_t obs[3] = {0};
            if (arenaos_syscall2(SYS_CAP_DESCRIBE, slot, (uint64_t)obs) != 0) {
                arenaos_debug_write("[arenaos_entry] FATAL: Capability describe failed\n", 50);
                return -1;
            }
            const CapDesc *want = &s_verified_caps[slot - 1];
            if (obs[0] != want->kind || obs[2] != want->rights) {
                arenaos_debug_write("[arenaos_entry] FATAL: Capability kind/rights mismatch\n", 55);
                return -1;
            }
        }
    }
    return 0;
}

/*
 * Phase 4: Graphical Application Contract Verification.
 */
static int verify_graphical_profile(void) {
    /*
     * For native graphical applications running under Desktop:
     * Slot 1: Desktop BadgedEndpoint (kind 12, WRITE rights)
     * Slot 2: Window Surface SharedRegion (kind 7, READ | WRITE rights)
     * Slot 3: Notification (kind 3, WRITE rights) for timer wakeups
     */
    if (s_cap_count < 3) {
        arenaos_debug_write("[arenaos_entry] FATAL: Insufficient capability count for graphical profile\n", 75);
        return -1;
    }

    if (s_verified_caps[0].slot != SERVICE_ENDPOINT ||
        s_verified_caps[0].kind != CAP_KIND_BADGED_ENDPOINT ||
        (s_verified_caps[0].rights & RIGHT_WRITE) == 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 1 is not a valid desktop endpoint\n", 63);
        return -1;
    }

    if (s_verified_caps[1].slot != SURFACE_SLOT ||
        s_verified_caps[1].kind != CAP_KIND_SHARED_REGION ||
        (s_verified_caps[1].rights & (RIGHT_READ | RIGHT_WRITE)) != (RIGHT_READ | RIGHT_WRITE)) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 2 is not a valid read/write surface\n", 65);
        return -1;
    }

    if (s_verified_caps[2].slot != CLOCK_SLOT ||
        s_verified_caps[2].kind != CAP_KIND_NOTIFICATION ||
        (s_verified_caps[2].rights & RIGHT_WRITE) == 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 3 is not a valid notification capability\n", 70);
        return -1;
    }

    return 0;
}

void arenaos_entry(void) {
    /* Step 1: Snapshot and ingest startup record from Slot 0 */
    if (ingest_startup_page() != 0) {
        arenaos_exit(122);
    }

    /* Step 2: Parse and canonicalize ARST v2 record */
    if (parse_and_validate_record() != 0) {
        arenaos_exit(122);
    }

    /* Step 3: Verify live capability table against verified record */
    if (verify_live_inventory() != 0) {
        arenaos_exit(122);
    }

    /* Step 4: Verify graphical application profile */
    if (verify_graphical_profile() != 0) {
        arenaos_exit(122);
    }

    /* Step 5: Initialize hardened memory heap scaffolding */
    arenaos_memory_init();

    /* Step 6: Run application main */
    int rc = main(s_argc, s_argv_ptrs);

    /* Step 7: Terminate cleanly */
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
