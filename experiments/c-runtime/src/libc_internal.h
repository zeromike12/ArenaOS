/*
 * Internal helpers shared by the C2 libc subset (not installed in the SDK).
 */
#ifndef ARENA_LIBC_INTERNAL_H
#define ARENA_LIBC_INTERNAL_H

#include <stddef.h>
#include <stdint.h>

/* Per-thread errno storage (errno.c). */
void arena_c_set_errno(int value);

/* Map a negative runtime or kernel status to the errno value the libc subset
 * reports. Unknown statuses map to EIO. */
int arena_c_errno_for(long status);

/* Checked monotonic clock read. Returns 0 and *us_out, or a negative status
 * (the plain arena_clock_us() would hide a failure as zero). */
int arena_c_clock_checked(uint64_t *us_out);

/* Standard-stream objects (stdio.c). */
struct arena_file;
struct arena_file *arena_stdin_object(void);
struct arena_file *arena_stdout_object(void);
struct arena_file *arena_stderr_object(void);

/* Process-exit hook run before arena_exit (exit.c). */
void arena_c_run_exit_handlers(void);

/* Per-thread errno read (errno.c) and the strerror text table. */
int arena_c_get_errno(void);
const char *arena_strerror_text(int errnum);

#endif /* ARENA_LIBC_INTERNAL_H */
