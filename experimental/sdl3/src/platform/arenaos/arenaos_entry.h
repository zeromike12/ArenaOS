/*
  arenaos_entry.h: Canonical ARST v2 Startup Specification & Parser Interface
  Conforms to ADR-0083, ADR-0086, and userspace/arena-platform/src/startup.rs
*/

#ifndef ARENAOS_ENTRY_H
#define ARENAOS_ENTRY_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define STARTUP_BLOCK_BYTES 4096u
#define STARTUP_HEADER_BYTES 128u
#define STARTUP_INSTANCE_SLOTS 32u
#define STARTUP_CAPABILITY_MAX 7u
#define STARTUP_ARGUMENT_MAX 32u
#define STARTUP_ENVIRONMENT_MAX 32u
#define STARTUP_STRING_BYTES_MAX 3072u
#define STARTUP_CAPABILITY_DESCRIPTOR_BYTES 16u
#define STARTUP_STRING_DESCRIPTOR_BYTES 8u
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

/* ARST Capability Roles */
#define CAP_ROLE_CURRENT_DIR 1u
#define CAP_ROLE_STDIN 2u
#define CAP_ROLE_STDOUT 3u
#define CAP_ROLE_STDERR 4u
#define CAP_ROLE_OTHER 5u
#define CAP_ROLE_STANDARD_STREAM_SET 6u
#define CAP_ROLE_STREAM_WAKE 7u
#define CAP_ROLE_SYNC_DOMAIN 8u

typedef struct {
    uint16_t slot;
    uint8_t role;
    uint8_t kind;
    uint32_t rights;
} arenaos_cap_desc_t;

typedef struct {
    uint32_t total_bytes;
    uint32_t flags;
    uint8_t application_id[32];
    uint16_t instance_slot;
    uint64_t instance_generation;
    uint16_t argc;
    uint16_t envc;
    uint16_t cap_count;
    arenaos_cap_desc_t capabilities[STARTUP_CAPABILITY_MAX];
    uint16_t cwd;
    uint16_t stdin_desc;
    uint16_t stdout_desc;
    uint16_t stderr_desc;
    uint64_t entry;
    uint64_t load_base;
    uint64_t clock_us;
} arenaos_startup_record_t;

/*
 * Canonical ARST v2 Record Validator and Parser.
 * Validates descriptive header bytes, padding, bounds, capabilities, and contiguous strings.
 * Copies arguments into argv_ptrs/string_store and environment into envp_ptrs/string_store.
 * Returns 0 on success, or negative error code on failure.
 */
int arenaos_validate_and_parse_arst(
    const uint8_t page[STARTUP_BLOCK_BYTES],
    arenaos_startup_record_t *out_rec,
    char *string_store,
    size_t string_store_capacity,
    char *argv_ptrs[STARTUP_ARGUMENT_MAX + 1],
    char *envp_ptrs[STARTUP_ENVIRONMENT_MAX + 1]
);

#endif /* ARENAOS_ENTRY_H */
