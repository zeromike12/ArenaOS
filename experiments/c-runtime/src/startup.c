/*
 * ARST v2 startup gate (C1.1 / C1.2).
 *
 * A C port of userspace/arena-platform/src/startup.rs and the entry gate in
 * userspace/arena-runtime/src/startup.rs. The order is the Rust order:
 *   1. slot 0 must be a SharedRegion with exactly READ|DESTROY and one page
 *   2. map it read-only, copy the page into private BSS, unmap, DESTROY slot 0
 *   3. parse the private copy with every canonical-layout and role rule
 *   4. verify the LIVE capability table: every listed cap present with the
 *      listed kind and rights, every unlisted slot empty
 * Only then does the application run. Nothing here trusts a value that the
 * kernel has not confirmed, and nothing allocates: the gate runs before the
 * heap and before the stack switch, so it uses only static storage and short
 * local frames.
 *
 * No record (slot 0 empty) is not an error here: legacy scratch images run
 * without one. An application that requires a record checks
 * arena_startup()->present itself.
 */
#include <stddef.h>
#include <stdint.h>

#include "arena/abi.h"
#include "arena/rt.h"
#include "arena/string.h"
#include "internal.h"

#define STARTUP_SLOT 0u
#define USER_ADDRESS_END 0x0000800000000000ULL
#define KNOWN_FLAGS                                                                \
    (ARENA_ARST_FLAG_MULTI_INSTANCE | ARENA_ARST_FLAG_BACKGROUND |                 \
     ARENA_ARST_FLAG_HEADLESS | ARENA_ARST_FLAG_STANDARD_STREAMS |                 \
     ARENA_ARST_FLAG_NATIVE_SYNC)
#define ID_BYTES 32u
#define RIGHTS_MASK 0xFu
#define RIGHT_RW (ARENA_RIGHT_READ | ARENA_RIGHT_WRITE)
#define CAP_KIND_BADGED_ENDPOINT 12u
#define CAP_KIND_RTC 13u

static uint8_t snapshot[ARENA_ARST_BLOCK_BYTES] __attribute__((aligned(4096)));
static arena_startup_t info;
static int gate_entered;

/* Private, NUL-terminated copies of the verified argv and envp strings. The
 * snapshot is not NUL-terminated per string, so the copies are required. */
static char string_store[ARENA_ARST_STRING_BYTES_MAX + ARENA_ARST_ARGUMENT_MAX +
                         ARENA_ARST_ENVIRONMENT_MAX + 1];
static char *argv_ptrs[ARENA_ARST_ARGUMENT_MAX + 1];
static char *envp_ptrs[ARENA_ARST_ENVIRONMENT_MAX + 1];

const arena_startup_t *arena_startup(void) {
    return &info;
}

static uint16_t rd16(const uint8_t *p, size_t at) {
    return (uint16_t)(p[at] | ((uint16_t)p[at + 1] << 8));
}

static uint32_t rd32(const uint8_t *p, size_t at) {
    return (uint32_t)p[at] | ((uint32_t)p[at + 1] << 8) | ((uint32_t)p[at + 2] << 16) |
           ((uint32_t)p[at + 3] << 24);
}

static uint64_t rd64(const uint8_t *p, size_t at) {
    return (uint64_t)rd32(p, at) | ((uint64_t)rd32(p, at + 4) << 32);
}

static int all_zero(const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (p[i] != 0) {
            return 0;
        }
    }
    return 1;
}

static int64_t sys_describe(uint64_t slot, uint64_t out[3]) {
    out[0] = out[1] = out[2] = 0;
    return arena_syscall6(ARENA_SYS_CAP_DESCRIBE, slot, (uint64_t)(uintptr_t)out, 0, 0, 0, 0);
}

/* userspace/arena-platform startup.rs valid_cap_kind: IMAGE..=RTC, or the
 * SyncDomain kind 16. */
static int valid_cap_kind(uint8_t kind) {
    return (kind >= 1u && kind <= CAP_KIND_RTC) || kind == ARENA_CAP_KIND_SYNC_DOMAIN;
}

static int valid_app_id(const uint8_t *id) {
    size_t end = 0;
    while (end < ID_BYTES && id[end] != 0) {
        end++;
    }
    if (end < 1 || end > 31 || !all_zero(id + end, ID_BYTES - end)) {
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

static int optional_index(const uint8_t *page, size_t at, unsigned cap_count, unsigned *out) {
    uint16_t v = rd16(page, at);
    if (v == ARENA_ARST_NONE) {
        *out = ARENA_ARST_NONE;
        return 0;
    }
    if (v >= cap_count) {
        return -1;
    }
    *out = v;
    return 0;
}

/* Validate one role's descriptor against its exact kind and rights. */
static int role_ok(uint8_t role, uint8_t kind, uint32_t rights) {
    switch (role) {
    case ARENA_ARST_ROLE_CURRENT_DIRECTORY:
        return kind == CAP_KIND_BADGED_ENDPOINT && (rights & ARENA_RIGHT_WRITE) != 0;
    case ARENA_ARST_ROLE_STANDARD_STREAM_SET:
        return kind == ARENA_CAP_KIND_SHARED_REGION && rights == RIGHT_RW;
    case ARENA_ARST_ROLE_STREAM_WAKE:
        return kind == ARENA_CAP_KIND_NOTIFICATION && rights == ARENA_RIGHT_WRITE;
    case ARENA_ARST_ROLE_SYNC_DOMAIN:
        return kind == ARENA_CAP_KIND_SYNC_DOMAIN && rights == RIGHT_RW;
    default:
        return 1;
    }
}

/* Parse and canonicalize one ARST v2 page. Returns 0 or -1. */
static int parse_record(const uint8_t *page) {
    if (page[0] != 'A' || page[1] != 'R' || page[2] != 'S' || page[3] != 'T') {
        return -1;
    }
    if (rd16(page, 4) != ARENA_ARST_VERSION || rd16(page, 6) != ARENA_ARST_HEADER_BYTES) {
        return -1;
    }
    if (page[50] || page[51] || page[66] || page[67] || page[100] || page[101] ||
        page[102] || page[103]) {
        return -1;
    }
    uint32_t total = rd32(page, 8);
    uint32_t flags = rd32(page, 12);
    uint16_t argc = rd16(page, 60);
    uint16_t envc = rd16(page, 62);
    uint16_t capc = rd16(page, 64);
    uint32_t caps_off = rd32(page, 76);
    uint32_t args_off = rd32(page, 68);
    uint32_t env_off = rd32(page, 72);
    uint32_t strings_off = rd32(page, 80);
    uint32_t strings_len = rd32(page, 84);
    uint16_t instance_slot = rd16(page, 48);
    uint64_t generation = rd64(page, 52);
    if (total < ARENA_ARST_HEADER_BYTES || total > ARENA_ARST_BLOCK_BYTES || argc == 0 ||
        argc > ARENA_ARST_ARGUMENT_MAX || envc > ARENA_ARST_ENVIRONMENT_MAX ||
        capc > ARENA_ARST_CAPABILITY_MAX || strings_len > ARENA_ARST_STRING_BYTES_MAX ||
        instance_slot >= 32u || generation == 0) {
        return -1;
    }
    if (flags & ~KNOWN_FLAGS) {
        return -1;
    }
    if (!valid_app_id(page + 16)) {
        return -1;
    }
    uint32_t expect_args = ARENA_ARST_HEADER_BYTES + (uint32_t)capc * 16u;
    uint32_t expect_env = expect_args + (uint32_t)argc * 8u;
    uint32_t expect_strings = expect_env + (uint32_t)envc * 8u;
    if (caps_off != ARENA_ARST_HEADER_BYTES || args_off != expect_args || env_off != expect_env ||
        strings_off != expect_strings || (uint64_t)strings_off + strings_len != total) {
        return -1;
    }
    if (!all_zero(page + total, ARENA_ARST_BLOCK_BYTES - total)) {
        return -1;
    }
    uint64_t entry = rd64(page, 104);
    uint64_t load_base = rd64(page, 112);
    if (rd32(page, 96) != ARENA_ARST_BLOCK_BYTES || entry == 0 || load_base == 0 ||
        (load_base & (ARENA_ARST_BLOCK_BYTES - 1u)) != 0 || entry < load_base ||
        entry >= USER_ADDRESS_END || load_base >= USER_ADDRESS_END) {
        return -1;
    }
    unsigned cwd_ref, stdin_ref, stdout_ref, stderr_ref;
    if (optional_index(page, 88, capc, &cwd_ref) || optional_index(page, 90, capc, &stdin_ref) ||
        optional_index(page, 92, capc, &stdout_ref) ||
        optional_index(page, 94, capc, &stderr_ref)) {
        return -1;
    }

    /* Capability descriptors: exact slot, valid kind/role, nonzero rights,
     * canonical padding, and one descriptor per exclusive role. */
    unsigned found_cwd = ARENA_ARST_NONE, found_in = ARENA_ARST_NONE,
             found_out = ARENA_ARST_NONE, found_err = ARENA_ARST_NONE,
             found_set = ARENA_ARST_NONE, found_wake = ARENA_ARST_NONE,
             found_sync = ARENA_ARST_NONE;
    for (unsigned i = 0; i < capc; i++) {
        size_t at = ARENA_ARST_HEADER_BYTES + (size_t)i * 16u;
        uint16_t slot = rd16(page, at);
        uint8_t role = page[at + 2];
        uint8_t kind = page[at + 3];
        uint32_t rights = rd32(page, at + 4);
        if (slot != i + 1u || !valid_cap_kind(kind) || rights == 0 || (rights & ~RIGHTS_MASK)) {
            return -1;
        }
        if (role < ARENA_ARST_ROLE_CURRENT_DIRECTORY || role > ARENA_ARST_ROLE_SYNC_DOMAIN ||
            !role_ok(role, kind, rights)) {
            return -1;
        }
        if (!all_zero(page + at + 8, 8)) {
            return -1;
        }
        unsigned *slotp = NULL;
        switch (role) {
        case ARENA_ARST_ROLE_CURRENT_DIRECTORY: slotp = &found_cwd; break;
        case ARENA_ARST_ROLE_STANDARD_INPUT: slotp = &found_in; break;
        case ARENA_ARST_ROLE_STANDARD_OUTPUT: slotp = &found_out; break;
        case ARENA_ARST_ROLE_STANDARD_ERROR: slotp = &found_err; break;
        case ARENA_ARST_ROLE_STANDARD_STREAM_SET: slotp = &found_set; break;
        case ARENA_ARST_ROLE_STREAM_WAKE: slotp = &found_wake; break;
        case ARENA_ARST_ROLE_SYNC_DOMAIN: slotp = &found_sync; break;
        default: break; /* ARENA_ARST_ROLE_OTHER: any number, no role slot */
        }
        if (slotp != NULL) {
            if (*slotp != ARENA_ARST_NONE) {
                return -1;
            }
            *slotp = i;
        }
    }
    if (found_cwd != cwd_ref) {
        return -1;
    }
    if (found_set != ARENA_ARST_NONE) {
        if (stdin_ref != found_set || stdout_ref != found_set || stderr_ref != found_set ||
            found_in != ARENA_ARST_NONE || found_out != ARENA_ARST_NONE ||
            found_err != ARENA_ARST_NONE || found_wake == ARENA_ARST_NONE) {
            return -1;
        }
    } else if (found_in != stdin_ref || found_out != stdout_ref || found_err != stderr_ref ||
               found_wake != ARENA_ARST_NONE) {
        return -1;
    }
    int streams = (flags & ARENA_ARST_FLAG_STANDARD_STREAMS) != 0;
    if (streams != (found_set != ARENA_ARST_NONE) || streams != (found_wake != ARENA_ARST_NONE)) {
        return -1;
    }
    if (((flags & ARENA_ARST_FLAG_NATIVE_SYNC) != 0) != (found_sync != ARENA_ARST_NONE)) {
        return -1;
    }

    /* Strings: canonical contiguous offsets, no embedded NUL, first argument
     * and every environment entry nonempty. */
    uint32_t expected = 0;
    for (unsigned pass = 0; pass < 2; pass++) {
        uint32_t table = pass == 0 ? args_off : env_off;
        uint32_t count = pass == 0 ? argc : envc;
        for (uint32_t i = 0; i < count; i++) {
            size_t at = table + (size_t)i * 8u;
            uint32_t off = rd32(page, at);
            uint32_t len = rd32(page, at + 4);
            if (off != expected || len > strings_len || off + len > strings_len) {
                return -1;
            }
            const uint8_t *s = page + strings_off + off;
            int has_nul = 0;
            for (uint32_t j = 0; j < len; j++) {
                if (s[j] == 0) {
                    has_nul = 1;
                    break;
                }
            }
            if (has_nul) {
                return -1;
            }
            if (len == 0 && (pass == 1 || i == 0)) {
                return -1;
            }
            expected = off + len;
        }
    }
    if (expected != strings_len) {
        return -1;
    }

    /* Accepted: publish the descriptive view. Slot values are still only
     * claims until arena_startup_gate's live check passes. */
    info.present = 1;
    info.flags = flags;
    info.instance_slot = instance_slot;
    info.instance_generation = generation;
    info.cap_count = capc;
    for (unsigned i = 0; i < capc; i++) {
        size_t at = ARENA_ARST_HEADER_BYTES + (size_t)i * 16u;
        info.caps[i].slot = rd16(page, at);
        info.caps[i].role = page[at + 2];
        info.caps[i].kind = page[at + 3];
        info.caps[i].rights = rd32(page, at + 4);
    }
    info.stream_set = found_set;
    info.stream_wake = found_wake;
    info.sync_domain = found_sync;
    info.notification = ARENA_ARST_NONE;
    for (unsigned i = 0; i < capc; i++) {
        if (info.caps[i].role == ARENA_ARST_ROLE_OTHER &&
            info.caps[i].kind == ARENA_CAP_KIND_NOTIFICATION && info.caps[i].rights == RIGHT_RW) {
            info.notification = i;
            break;
        }
    }

    /* Copy the strings into private NUL-terminated storage. */
    size_t pos = 0;
    for (unsigned pass = 0; pass < 2; pass++) {
        uint32_t table = pass == 0 ? args_off : env_off;
        uint32_t count = pass == 0 ? argc : envc;
        char **ptrs = pass == 0 ? argv_ptrs : envp_ptrs;
        for (uint32_t i = 0; i < count; i++) {
            size_t at = table + (size_t)i * 8u;
            uint32_t off = rd32(page, at);
            uint32_t len = rd32(page, at + 4);
            ptrs[i] = &string_store[pos];
            arena_memcpy(&string_store[pos], page + strings_off + off, len);
            string_store[pos + len] = '\0';
            pos += len + 1u;
        }
        ptrs[count] = NULL;
    }
    info.argc = argc;
    info.argv = argv_ptrs;
    info.envc = envc;
    info.envp = envp_ptrs;
    return 0;
}

/* Verify the live capability table against the verified record. */
static int verify_live(void) {
    for (uint32_t slot = 0; slot < ARENA_CAP_SLOTS; slot++) {
        int listed = slot > 0 && slot <= info.cap_count;
        int64_t occ = arena_syscall6(ARENA_SYS_CAP_OCCUPIED, slot, 0, 0, 0, 0, 0);
        if (occ != 0 && occ != 1) {
            return -1;
        }
        if (listed != (int)occ) {
            return -1; /* listed but empty, or occupied but unlisted */
        }
        if (listed) {
            uint64_t obs[3];
            const arena_cap_desc_t *want = &info.caps[slot - 1];
            if (want->slot != slot || sys_describe(slot, obs) != 0) {
                return -1;
            }
            if (obs[0] != want->kind || obs[2] != want->rights ||
                (obs[2] & ~(uint64_t)RIGHTS_MASK) != 0) {
                return -1;
            }
        }
    }
    return 0;
}

int arena_startup_gate(void) {
    if (gate_entered) {
        return ARENA_E_STARTUP;
    }
    gate_entered = 1;
    int64_t occ = arena_syscall6(ARENA_SYS_CAP_OCCUPIED, STARTUP_SLOT, 0, 0, 0, 0, 0);
    if (occ == 0) {
        return 0; /* no record granted: legacy image */
    }
    if (occ != 1) {
        return ARENA_E_STARTUP;
    }
    uint64_t obs[3];
    if (sys_describe(STARTUP_SLOT, obs) != 0) {
        return ARENA_E_STARTUP;
    }
    int discard = obs[0] == ARENA_CAP_KIND_SHARED_REGION && (obs[2] & ARENA_RIGHT_DESTROY);
    if (obs[0] != ARENA_CAP_KIND_SHARED_REGION || obs[2] != (ARENA_RIGHT_READ | ARENA_RIGHT_DESTROY)) {
        if (discard) {
            (void)arena_syscall6(ARENA_SYS_CAP_DESTROY, STARTUP_SLOT, 0, 0, 0, 0, 0);
        }
        return ARENA_E_STARTUP;
    }
    int64_t pages = arena_syscall6(ARENA_SYS_SHARED_PAGES, STARTUP_SLOT, 0, 0, 0, 0, 0);
    if (pages != 1) {
        (void)arena_syscall6(ARENA_SYS_CAP_DESTROY, STARTUP_SLOT, 0, 0, 0, 0, 0);
        return ARENA_E_STARTUP;
    }
    int64_t va = arena_syscall6(ARENA_SYS_SHARED_MAP, STARTUP_SLOT, 0, 0, 0, 0, 0);
    if (va <= 0 || ((uint64_t)va & (ARENA_ARST_BLOCK_BYTES - 1u)) != 0 ||
        (uint64_t)va + ARENA_ARST_BLOCK_BYTES > USER_ADDRESS_END) {
        if (va > 0) {
            (void)arena_syscall6(ARENA_SYS_SHARED_UNMAP, (uint64_t)va, 0, 0, 0, 0, 0);
        }
        (void)arena_syscall6(ARENA_SYS_CAP_DESTROY, STARTUP_SLOT, 0, 0, 0, 0, 0);
        return ARENA_E_STARTUP;
    }
    arena_memcpy(snapshot, (const void *)(uintptr_t)va, ARENA_ARST_BLOCK_BYTES);
    int64_t unmap = arena_syscall6(ARENA_SYS_SHARED_UNMAP, (uint64_t)va, 0, 0, 0, 0, 0);
    int64_t destroy = arena_syscall6(ARENA_SYS_CAP_DESTROY, STARTUP_SLOT, 0, 0, 0, 0, 0);
    if (unmap != 0 || destroy != 0) {
        return ARENA_E_STARTUP;
    }
    if (parse_record(snapshot) != 0) {
        return ARENA_E_STARTUP;
    }
    if (verify_live() != 0) {
        info.present = 0;
        return ARENA_E_STARTUP;
    }
    return 0;
}
