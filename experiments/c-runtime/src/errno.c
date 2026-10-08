/*
 * errno (C2.2). Per-thread through TLS, so each thread sees its own value.
 * The runtime maps every negative ARENA_E_* and kernel status onto the errno
 * subset in include/libc/errno.h. Unknown statuses map to EIO.
 */
#include <stddef.h>

#include "arena/rt.h"
#include "libc_internal.h"

static _Thread_local int arena_errno_value;

void arena_c_set_errno(int value) { arena_errno_value = value; }

int arena_c_get_errno(void) { return arena_errno_value; }

int *__arena_errno_location(void) { return &arena_errno_value; }

int arena_c_errno_for(long status) {
    switch (status) {
    case ARENA_E_NO_STARTUP: return 2;   /* ENOENT */
    case ARENA_E_STARTUP: return 22;     /* EINVAL */
    case ARENA_E_NO_CAP: return 1;       /* EPERM: capability not granted */
    case ARENA_E_NO_STREAMS: return 9;   /* EBADF: no stream grant */
    case ARENA_E_WOULD_BLOCK: return 11; /* EAGAIN */
    case ARENA_E_BROKEN_PIPE: return 32; /* EPIPE */
    case ARENA_E_CLOSED: return 9;       /* EBADF */
    case ARENA_E_CORRUPT: return 5;      /* EIO */
    case ARENA_E_INVALID: return 22;     /* EINVAL */
    case ARENA_E_STALE: return 22;       /* EINVAL */
    case ARENA_E_NOMEM: return 12;       /* ENOMEM */
    case ARENA_E_NO_WAITER: return 1;    /* EPERM: no wake notification granted */
    case ARENA_E_OVERFLOW: return 75;    /* EOVERFLOW */
    case ARENA_E_UNSUPPORTED: return 95; /* ENOTSUP */
    case -1: return 22;                  /* ARENA_STATUS_BAD_CALL */
    case -2: return 22;                  /* ARENA_STATUS_BAD_ARG */
    case -3: return 14;                  /* ARENA_STATUS_BAD_ADDRESS: EFAULT */
    case -4: return 16;                  /* ARENA_STATUS_BUSY: EBUSY */
    case -7: return 11;                  /* ARENA_STATUS_QUOTA: EAGAIN */
    case -8: return 110;                 /* ARENA_STATUS_TIMEOUT: ETIMEDOUT */
    default: return 5;                   /* EIO */
    }
}

const char *arena_strerror_text(int errnum) {
    switch (errnum) {
    case 0: return "Success";
    case 1: return "Operation not permitted";
    case 2: return "No such file or directory";
    case 5: return "Input/output error";
    case 7: return "Argument list too long";
    case 9: return "Bad file descriptor";
    case 11: return "Resource temporarily unavailable";
    case 12: return "Cannot allocate memory";
    case 13: return "Permission denied";
    case 14: return "Bad address";
    case 16: return "Device or resource busy";
    case 17: return "File exists";
    case 22: return "Invalid argument";
    case 23: return "Too many open files in system";
    case 24: return "Too many open files";
    case 28: return "No space left on device";
    case 29: return "Illegal seek";
    case 32: return "Broken pipe";
    case 33: return "Numerical argument out of domain";
    case 34: return "Numerical result out of range";
    case 38: return "Function not implemented";
    case 75: return "Value too large for defined data type";
    case 95: return "Operation not supported";
    case 110: return "Connection timed out";
    default: return "Unknown error";
    }
}
