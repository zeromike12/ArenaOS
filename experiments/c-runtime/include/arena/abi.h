/*
 * ArenaOS native syscall and startup ABI - C mirror (EXPERIMENTAL, x86_64).
 *
 * Source of truth (checked by `run.py abi`, which fails on any constant that
 * is missing from, or disagrees with, the Rust mirror or kernel):
 *   userspace/abi.rs                       syscall numbers, statuses, rights
 *   userspace/arena-platform/src/startup.rs ARST v2 layout and roles
 *   userspace/arena-runtime/src/streams.rs  ASTR stream page layout
 *
 * Calling convention (ADR-0017): RAX = call number; arguments in RDI, RSI,
 * RDX, R10, R8, R9. Result in RAX as a typed i64: 0 = OK, positive =
 * call-specific payload, negative = typed status. The kernel validates
 * unused trailing argument registers, so the wrapper always passes six
 * arguments and callers pass 0 for unused ones.
 */
#ifndef ARENA_ABI_H
#define ARENA_ABI_H

#include <stdint.h>

/* Syscall numbers and the raw syscall stub live in src/sysabi.h (internal). */

/* --- typed statuses (userspace/abi.rs) ----------------------------------- */
#define ARENA_STATUS_OK 0
#define ARENA_STATUS_BAD_CALL (-1)
#define ARENA_STATUS_BAD_ARG (-2)
#define ARENA_STATUS_BAD_ADDRESS (-3)
#define ARENA_STATUS_BUSY (-4)
#define ARENA_STATUS_SERVICE_GONE (-5)
#define ARENA_STATUS_CALLER_GONE (-6)
#define ARENA_STATUS_QUOTA (-7)
#define ARENA_STATUS_TIMEOUT (-8)

/* Kernel capability table size (userspace/abi.rs CAP_SLOTS). */
#define ARENA_CAP_SLOTS 128u

/* --- capability kinds and rights (userspace/abi.rs, startup.rs) ---------- */
#define ARENA_CAP_KIND_NOTIFICATION 3u
#define ARENA_CAP_KIND_PROCESS 4u
#define ARENA_CAP_KIND_SHARED_REGION 7u
#define ARENA_CAP_KIND_MEMORY_POOL 8u
#define ARENA_CAP_KIND_BADGED_ENDPOINT 12u
#define ARENA_CAP_KIND_VM_REGION 14u
#define ARENA_CAP_KIND_SYNC_DOMAIN 16u
#define ARENA_RIGHT_READ 1u
#define ARENA_RIGHT_WRITE 2u
#define ARENA_RIGHT_COPY 4u
#define ARENA_RIGHT_DESTROY 8u

/* --- VM protection bits (userspace/abi.rs) ------------------------------- */
#define ARENA_VM_PROT_READ 1u
#define ARENA_VM_PROT_WRITE 2u
#define ARENA_VM_PROT_EXEC 4u

/* --- startup ABI v2 (startup.rs) ----------------------------------------- */
#define ARENA_ARST_BLOCK_BYTES 4096u
#define ARENA_ARST_HEADER_BYTES 128u
#define ARENA_ARST_ARGUMENT_MAX 32u
#define ARENA_ARST_ENVIRONMENT_MAX 32u
#define ARENA_ARST_CAPABILITY_MAX 7u
#define ARENA_ARST_STRING_BYTES_MAX 3072u
#define ARENA_ARST_CAPABILITY_DESCRIPTOR_BYTES 16u
#define ARENA_ARST_STRING_DESCRIPTOR_BYTES 8u
#define ARENA_ARST_VERSION 2u
#define ARENA_ARST_NONE 0xFFFFu
#define ARENA_ARST_FLAG_MULTI_INSTANCE 1u
#define ARENA_ARST_FLAG_BACKGROUND 2u
#define ARENA_ARST_FLAG_HEADLESS 4u
#define ARENA_ARST_FLAG_STANDARD_STREAMS 8u
#define ARENA_ARST_FLAG_NATIVE_SYNC 16u
#define ARENA_ARST_ROLE_CURRENT_DIRECTORY 1u
#define ARENA_ARST_ROLE_STANDARD_INPUT 2u
#define ARENA_ARST_ROLE_STANDARD_OUTPUT 3u
#define ARENA_ARST_ROLE_STANDARD_ERROR 4u
#define ARENA_ARST_ROLE_OTHER 5u
#define ARENA_ARST_ROLE_STANDARD_STREAM_SET 6u
#define ARENA_ARST_ROLE_STREAM_WAKE 7u
#define ARENA_ARST_ROLE_SYNC_DOMAIN 8u

/* --- stream page layout (streams.rs) ------------------------------------- */
#define ARENA_STREAM_PAGE_BYTES 4096u
#define ARENA_STREAM_CHANNELS 3u
#define ARENA_STREAM_CAPACITY 768u
#define ARENA_STREAM_WAKE_BADGE (1u << 5)  /* app -> desktop wake (streams.rs) */
/* desktop -> app readiness badge on the app's own notification (desktop.rs
 * BADGE_STREAM_READY). Waiters treat every badge as a hint; this is documented
 * here so no runtime code relies on a specific value. */
#define ARENA_STREAM_READY_BADGE (1u << 2)
#define ARENA_STREAM_MAGIC 0x52545341u /* "ASTR", little-endian u32 */
#define ARENA_STREAM_VERSION 1u

/* --- scalars ------------------------------------------------------------- */
#define ARENA_PAGE_SIZE 4096u

#endif /* ARENA_ABI_H */
