/*
  ArenaOS Freestanding C Runtime Entry & Canonical ARST v2 Gate
  Conforms to ADR-0083, ADR-0086, and userspace/arena-platform/src/startup.rs.
*/

#include "arenaos_entry.h"

#ifndef ARENAOS_HOST_TEST
#include "arenaos_syscalls.h"

#define STACK_SIZE (64 * 1024)

/* 64 KiB 4096-aligned stack in .bss */
static uint8_t s_stack[STACK_SIZE] __attribute__((aligned(4096)));
/* 4 KiB 4096-aligned snapshot buffer in .bss */
static uint8_t s_startup_snapshot[STARTUP_BLOCK_BYTES] __attribute__((aligned(4096)));

static arenaos_startup_record_t s_startup_record;

/* String storage for argv and envp */
static char s_string_store[STARTUP_STRING_BYTES_MAX + STARTUP_ARGUMENT_MAX + STARTUP_ENVIRONMENT_MAX + 2];
static char *s_argv_ptrs[STARTUP_ARGUMENT_MAX + 1];
static char *s_envp_ptrs[STARTUP_ENVIRONMENT_MAX + 1];
static int s_argc = 0;

extern void arenaos_memory_init(void);
extern int main(int argc, char *argv[]);

#endif /* !ARENAOS_HOST_TEST */

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
 * Canonical ARST v2 Layout, Bounds, Capability, and String Validation.
 */
int arenaos_validate_and_parse_arst(
    const uint8_t page[STARTUP_BLOCK_BYTES],
    arenaos_startup_record_t *out_rec,
    char *string_store,
    size_t string_store_capacity,
    char *argv_ptrs[STARTUP_ARGUMENT_MAX + 1],
    char *envp_ptrs[STARTUP_ENVIRONMENT_MAX + 1]
) {
    if (!page || !out_rec) {
        return -1;
    }

    /* 1. Magic and version check */
    if (page[0] != 'A' || page[1] != 'R' || page[2] != 'S' || page[3] != 'T') {
        return -2; /* BadMagic */
    }
    if (rd16(page, 4) != STARTUP_VERSION || rd16(page, 6) != STARTUP_HEADER_BYTES) {
        return -3; /* UnsupportedVersion */
    }

    /* 2. Header padding bytes must be zero */
    if (page[50] || page[51] || page[66] || page[67] ||
        page[100] || page[101] || page[102] || page[103]) {
        return -4; /* NonCanonical */
    }

    /* 3. Header fields extraction */
    uint32_t total = rd32(page, 8);
    uint32_t flags = rd32(page, 12);
    uint16_t instance_slot = rd16(page, 48);
    uint64_t generation = rd64(page, 52);
    uint16_t argc = rd16(page, 60);
    uint16_t envc = rd16(page, 62);
    uint16_t cap_count = rd16(page, 64);
    uint32_t args_off = rd32(page, 68);
    uint32_t env_off = rd32(page, 72);
    uint32_t caps_off = rd32(page, 76);
    uint32_t strings_off = rd32(page, 80);
    uint32_t strings_len = rd32(page, 84);
    uint16_t cwd = rd16(page, 88);
    uint16_t stdin_idx = rd16(page, 90);
    uint16_t stdout_idx = rd16(page, 92);
    uint16_t stderr_idx = rd16(page, 94);
    uint32_t page_size = rd32(page, 96);
    uint64_t entry = rd64(page, 104);
    uint64_t load_base = rd64(page, 112);
    uint64_t clock_us = rd64(page, 120);

    /* 4. Bounds checks */
    if (total < STARTUP_HEADER_BYTES || total > STARTUP_BLOCK_BYTES ||
        argc == 0 || argc > STARTUP_ARGUMENT_MAX ||
        envc > STARTUP_ENVIRONMENT_MAX ||
        cap_count > STARTUP_CAPABILITY_MAX ||
        strings_len > STARTUP_STRING_BYTES_MAX ||
        instance_slot >= STARTUP_INSTANCE_SLOTS ||
        generation == 0) {
        return -5; /* Bounds */
    }

    if (flags & ~KNOWN_FLAGS) {
        return -6; /* BadFlags */
    }

    if (!valid_app_id(page + 16)) {
        return -7; /* BadIdentity */
    }

    if (page_size != STARTUP_BLOCK_BYTES || entry == 0 || load_base == 0 ||
        (load_base % STARTUP_BLOCK_BYTES) != 0 || entry < load_base ||
        entry >= USER_ADDRESS_END || load_base >= USER_ADDRESS_END) {
        return -8; /* BadEntry */
    }

    /* 5. Canonical table offsets */
    uint32_t expect_args = STARTUP_HEADER_BYTES + (uint32_t)cap_count * STARTUP_CAPABILITY_DESCRIPTOR_BYTES;
    uint32_t expect_env = expect_args + (uint32_t)argc * STARTUP_STRING_DESCRIPTOR_BYTES;
    uint32_t expect_strings = expect_env + (uint32_t)envc * STARTUP_STRING_DESCRIPTOR_BYTES;
    if (caps_off != STARTUP_HEADER_BYTES || args_off != expect_args ||
        env_off != expect_env || strings_off != expect_strings ||
        (uint64_t)strings_off + strings_len != (uint64_t)total) {
        return -9; /* NonCanonical */
    }

    /* Unused tail padding must be zero */
    if (!all_zero(page + total, STARTUP_BLOCK_BYTES - total)) {
        return -10; /* NonCanonical */
    }

    /* 6. Capability descriptors validation */
    int found_role[9] = {0};
    int stream_set_idx = -1;
    int stream_wake_idx = -1;
    int sync_domain_idx = -1;

    for (unsigned i = 0; i < cap_count; i++) {
        size_t at = STARTUP_HEADER_BYTES + (size_t)i * STARTUP_CAPABILITY_DESCRIPTOR_BYTES;
        uint16_t slot = rd16(page, at);
        uint8_t role = page[at + 2];
        uint8_t kind = page[at + 3];
        uint32_t rights = rd32(page, at + 4);

        if (slot != i + 1u || !valid_cap_kind(kind) || rights == 0 || (rights & ~RIGHTS_MASK)) {
            return -11; /* InvalidCapability */
        }
        if (role < 1u || role > 8u) {
            return -12; /* InvalidCapability */
        }
        if (!all_zero(page + at + 8, 8)) {
            return -13; /* NonCanonical */
        }

        /* Check role uniqueness (roles other than Other=5 must be unique) */
        if (role != CAP_ROLE_OTHER) {
            if (found_role[role]) {
                return -14; /* InvalidCapability - duplicate unique role */
            }
            found_role[role] = 1;
        }

        /* Role-specific kind and rights constraints */
        if (role == CAP_ROLE_CURRENT_DIR &&
            (kind != CAP_KIND_BADGED_ENDPOINT || (rights & RIGHT_WRITE) == 0)) {
            return -15; /* InvalidCapability */
        }
        if (role == CAP_ROLE_STANDARD_STREAM_SET) {
            if (kind != CAP_KIND_SHARED_REGION || rights != (RIGHT_READ | RIGHT_WRITE)) {
                return -16; /* InvalidCapability */
            }
            stream_set_idx = (int)i;
        }
        if (role == CAP_ROLE_STREAM_WAKE) {
            if (kind != CAP_KIND_NOTIFICATION || rights != RIGHT_WRITE) {
                return -17; /* InvalidCapability */
            }
            stream_wake_idx = (int)i;
        }
        if (role == CAP_ROLE_SYNC_DOMAIN) {
            if (kind != CAP_KIND_SYNC_DOMAIN || rights != (RIGHT_READ | RIGHT_WRITE)) {
                return -18; /* InvalidCapability */
            }
            sync_domain_idx = (int)i;
        }

        out_rec->capabilities[i].slot = slot;
        out_rec->capabilities[i].role = role;
        out_rec->capabilities[i].kind = kind;
        out_rec->capabilities[i].rights = rights;
    }

    /* Validate reference descriptors */
    if (cwd != STARTUP_NONE) {
        if (cwd >= cap_count || out_rec->capabilities[cwd].role != CAP_ROLE_CURRENT_DIR) {
            return -19; /* InvalidRoleReference */
        }
    } else if (found_role[CAP_ROLE_CURRENT_DIR]) {
        return -20; /* InvalidRoleReference */
    }

    bool streams_requested = (flags & FLAG_STANDARD_STREAMS) != 0;
    if (streams_requested) {
        if (stream_set_idx < 0 || stream_wake_idx < 0) {
            return -21; /* InvalidRoleReference */
        }
        if (stdin_idx != (uint16_t)stream_set_idx ||
            stdout_idx != (uint16_t)stream_set_idx ||
            stderr_idx != (uint16_t)stream_set_idx) {
            return -22; /* InvalidRoleReference */
        }
    } else {
        if (stream_set_idx >= 0 || stream_wake_idx >= 0 ||
            stdin_idx != STARTUP_NONE || stdout_idx != STARTUP_NONE || stderr_idx != STARTUP_NONE) {
            return -23; /* InvalidRoleReference */
        }
    }

    bool sync_requested = (flags & FLAG_NATIVE_SYNC) != 0;
    if (sync_requested != (sync_domain_idx >= 0)) {
        return -24; /* InvalidRoleReference */
    }

    /* 7. Validate String Table and Copy Arguments/Environment */
    size_t required_store = (size_t)strings_len + (size_t)argc + (size_t)envc + 2u;
    if (string_store && string_store_capacity < required_store) {
        return -25; /* Destination buffer overflow refused */
    }

    uint32_t expected_offset = 0;
    size_t store_pos = 0;

    /* Validate and copy arguments */
    for (uint32_t i = 0; i < argc; i++) {
        size_t at = args_off + (size_t)i * STARTUP_STRING_DESCRIPTOR_BYTES;
        uint32_t off = rd32(page, at);
        uint32_t len = rd32(page, at + 4);

        if (off != expected_offset) {
            return -26; /* NonCanonical contiguous string order violation */
        }
        if ((uint64_t)off + len > strings_len) {
            return -27; /* Bounds: extends past strings_len */
        }
        if (i == 0 && len == 0) {
            return -28; /* InvalidString: argv[0] cannot be empty */
        }

        const uint8_t *str_bytes = page + strings_off + off;
        for (uint32_t k = 0; k < len; k++) {
            if (str_bytes[k] == 0) {
                return -29; /* InvalidString: embedded NUL prohibited */
            }
        }

        if (string_store && argv_ptrs) {
            argv_ptrs[i] = &string_store[store_pos];
            for (uint32_t k = 0; k < len; k++) {
                string_store[store_pos + k] = (char)str_bytes[k];
            }
            string_store[store_pos + len] = '\0';
            store_pos += len + 1u;
        }

        expected_offset = off + len;
    }
    if (argv_ptrs) {
        argv_ptrs[argc] = NULL;
    }

    /* Validate and copy environment */
    for (uint32_t i = 0; i < envc; i++) {
        size_t at = env_off + (size_t)i * STARTUP_STRING_DESCRIPTOR_BYTES;
        uint32_t off = rd32(page, at);
        uint32_t len = rd32(page, at + 4);

        if (off != expected_offset) {
            return -30; /* NonCanonical contiguous string order violation */
        }
        if ((uint64_t)off + len > strings_len) {
            return -31; /* Bounds: extends past strings_len */
        }
        if (len == 0) {
            return -32; /* InvalidString: environment string cannot be empty */
        }

        const uint8_t *str_bytes = page + strings_off + off;
        for (uint32_t k = 0; k < len; k++) {
            if (str_bytes[k] == 0) {
                return -33; /* InvalidString: embedded NUL prohibited */
            }
        }

        if (string_store && envp_ptrs) {
            envp_ptrs[i] = &string_store[store_pos];
            for (uint32_t k = 0; k < len; k++) {
                string_store[store_pos + k] = (char)str_bytes[k];
            }
            string_store[store_pos + len] = '\0';
            store_pos += len + 1u;
        }

        expected_offset = off + len;
    }
    if (envp_ptrs) {
        envp_ptrs[envc] = NULL;
    }

    /* Whole strings section must be accounted for */
    if (expected_offset != strings_len) {
        return -34; /* NonCanonical: incomplete or trailing string bytes */
    }

    /* Populate output record */
    out_rec->total_bytes = total;
    out_rec->flags = flags;
    for (size_t i = 0; i < 32; i++) {
        out_rec->application_id[i] = page[16 + i];
    }
    out_rec->instance_slot = instance_slot;
    out_rec->instance_generation = generation;
    out_rec->argc = argc;
    out_rec->envc = envc;
    out_rec->cap_count = cap_count;
    out_rec->cwd = cwd;
    out_rec->stdin_desc = stdin_idx;
    out_rec->stdout_desc = stdout_idx;
    out_rec->stderr_desc = stderr_idx;
    out_rec->entry = entry;
    out_rec->load_base = load_base;
    out_rec->clock_us = clock_us;

    return 0;
}

#ifndef ARENAOS_HOST_TEST

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
 * Phase 2: Live Capability Inventory Verification.
 */
static int verify_live_inventory(void) {
    for (uint32_t slot = 0; slot < 128u; slot++) {
        int listed = (slot > 0 && slot <= s_startup_record.cap_count);
        int64_t occ = arenaos_syscall1(SYS_CAP_OCCUPIED, slot);
        if (occ != 0 && occ != 1) {
            arenaos_debug_write("[arenaos_entry] FATAL: Capability occupied query failed\n", 56);
            return -1;
        }
        if (listed != (int)occ) {
            arenaos_debug_write("[arenaos_entry] FATAL: Capability inventory mismatch against ARST\n", 66);
            return -1;
        }
        if (listed) {
            uint64_t obs[3] = {0};
            if (arenaos_syscall2(SYS_CAP_DESCRIBE, slot, (uint64_t)obs) != 0) {
                arenaos_debug_write("[arenaos_entry] FATAL: Capability describe failed\n", 50);
                return -1;
            }
            const arenaos_cap_desc_t *want = &s_startup_record.capabilities[slot - 1];
            if (obs[0] != want->kind || obs[2] != want->rights) {
                arenaos_debug_write("[arenaos_entry] FATAL: Capability kind/rights mismatch\n", 55);
                return -1;
            }
        }
    }
    return 0;
}

/*
 * Phase 3: Graphical Application Contract Verification.
 */
static int verify_graphical_profile(void) {
    /*
     * For native graphical applications running under Desktop:
     * Slot 1: Desktop BadgedEndpoint (kind 12, WRITE rights)
     * Slot 2: Window Surface SharedRegion (kind 7, READ | WRITE rights)
     * Slot 3: Notification (kind 3, READ | WRITE rights) for timer wakeups & waits
     */
    if (s_startup_record.cap_count < 3) {
        arenaos_debug_write("[arenaos_entry] FATAL: Insufficient capability count for graphical profile\n", 75);
        return -1;
    }

    if (s_startup_record.capabilities[0].slot != SERVICE_ENDPOINT ||
        s_startup_record.capabilities[0].kind != CAP_KIND_BADGED_ENDPOINT ||
        (s_startup_record.capabilities[0].rights & RIGHT_WRITE) == 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 1 is not a valid desktop endpoint\n", 63);
        return -1;
    }

    if (s_startup_record.capabilities[1].slot != SURFACE_SLOT ||
        s_startup_record.capabilities[1].kind != CAP_KIND_SHARED_REGION ||
        (s_startup_record.capabilities[1].rights & (RIGHT_READ | RIGHT_WRITE)) != (RIGHT_READ | RIGHT_WRITE)) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 2 is not a valid read/write surface\n", 65);
        return -1;
    }

    if (s_startup_record.capabilities[2].slot != CLOCK_SLOT ||
        s_startup_record.capabilities[2].kind != CAP_KIND_NOTIFICATION ||
        (s_startup_record.capabilities[2].rights & (RIGHT_READ | RIGHT_WRITE)) != (RIGHT_READ | RIGHT_WRITE)) {
        arenaos_debug_write("[arenaos_entry] FATAL: Slot 3 does not possess required READ|WRITE rights\n", 74);
        return -1;
    }

    return 0;
}

void arenaos_entry(void) {
    /* Step 1: Ingest and snapshot startup record from Slot 0 */
    if (ingest_startup_page() != 0) {
        arenaos_exit(122);
    }

    /* Step 2: Canonical ARST v2 Layout, Bounds, and Contiguous String Validation */
    if (arenaos_validate_and_parse_arst(
            s_startup_snapshot,
            &s_startup_record,
            s_string_store,
            sizeof(s_string_store),
            s_argv_ptrs,
            s_envp_ptrs
        ) != 0) {
        arenaos_debug_write("[arenaos_entry] FATAL: ARST v2 validation failed\n", 49);
        arenaos_exit(122);
    }
    s_argc = (int)s_startup_record.argc;

    /* Step 3: Exact 128-slot live capability inventory check via SYS_CAP_DESCRIBE */
    if (verify_live_inventory() != 0) {
        arenaos_exit(122);
    }

    /* Step 4: Verify graphical application profile contract */
    if (verify_graphical_profile() != 0) {
        arenaos_exit(122);
    }

    /* Step 5: Initialize freestanding memory allocator */
    arenaos_memory_init();

    /* Step 6: Transfer control to application main() */
    arenaos_debug_write("[arenaos_entry] Startup validation and capability inventory verified; entering main\n", 86);
    int rc = main(s_argc, s_argv_ptrs);

    /* Step 7: Clean process finish */
    arenaos_exit(rc);
}

/*
 * _start: Naked Assembly Entry Point for Freestanding Dynamic User Executable.
 */
__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "movq %0, %%rsp\n\t"
        "andq $-16, %%rsp\n\t"
        "call arenaos_entry\n\t"
        "1: jmp 1b\n\t"
        :
        : "r" (s_stack + STACK_SIZE)
        : "memory"
    );
}

#endif /* !ARENAOS_HOST_TEST */
