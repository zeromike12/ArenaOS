/* ArenaOS C2 errno (EXPERIMENTAL). errno is per thread (TLS). The codes are the subset the runtime sets. */
#ifndef ARENA_LIBC_ERRNO_H
#define ARENA_LIBC_ERRNO_H

int *__arena_errno_location(void);
#define errno (*__arena_errno_location())

#define EPERM 1
#define ENOENT 2
#define EIO 5
#define E2BIG 7
#define EBADF 9
#define EAGAIN 11
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EBUSY 16
#define EEXIST 17
#define EINVAL 22
#define ENFILE 23
#define EMFILE 24
#define ENOSPC 28
#define ESPIPE 29
#define EPIPE 32
#define EDOM 33
#define ERANGE 34
#define ENOSYS 38
#define EOVERFLOW 75
#define ENOTSUP 95
#define EOPNOTSUPP 95
#define ETIMEDOUT 110

#endif /* ARENA_LIBC_ERRNO_H */
