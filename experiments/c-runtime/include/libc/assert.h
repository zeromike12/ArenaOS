/* ArenaOS C2 assert (EXPERIMENTAL). On failure: message to stderr, then abort (status 134). */
#ifndef ARENA_LIBC_ASSERT_H
#define ARENA_LIBC_ASSERT_H

#ifdef NDEBUG
#define assert(expr) ((void)0)
#else
void __arena_assert_fail(const char *expr, const char *file, int line) __attribute__((noreturn));
#define assert(expr) ((expr) ? (void)0 : __arena_assert_fail(#expr, __FILE__, __LINE__))
#endif

#endif /* ARENA_LIBC_ASSERT_H */
