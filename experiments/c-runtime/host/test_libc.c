/*
 * C2.2 host tests for the libc subset (HOST-ONLY, ASan + UBSan).
 *
 * Differential checks run the arena_* implementation and the host glibc on the
 * same inputs and require identical results (bytes, return values, end pointers,
 * errno where both define it). Refusal checks require the documented errno and no
 * partial output. Allocation checks cover alignment, refusal of invalid pointers,
 * and accounting.
 *
 * This file does NOT include the SDK's libc/time.h (glibc's <stdlib.h> already
 * defines struct timespec on the host). It declares the few prototypes it needs.
 */
#include <errno.h>
#include <limits.h>
#include <pthread.h>
#include <sys/types.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "arena/rt.h"
#include "arena/string.h"
#include "internal.h"
#include "libc_internal.h"

/* arena_* prototypes (implementations: stdio.c, libc_stdlib.c, libc_string.c, exit.c) */
int arena_snprintf(char *buf, size_t n, const char *fmt, ...);
int arena_vsnprintf(char *buf, size_t n, const char *fmt, va_list ap);
long arena_strtol(const char *s, char **end, int base);
unsigned long arena_strtoul(const char *s, char **end, int base);
long long arena_strtoll(const char *s, char **end, int base);
unsigned long long arena_strtoull(const char *s, char **end, int base);
void arena_qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *));
void *arena_bsearch(const void *key, const void *base, size_t n, size_t size,
                    int (*cmp)(const void *, const void *));
char *arena_strchr(const char *s, int c);
char *arena_strrchr(const char *s, int c);
char *arena_strstr(const char *hay, const char *needle);
char *arena_strpbrk(const char *s, const char *accept);
size_t arena_strspn(const char *s, const char *accept);
size_t arena_strcspn(const char *s, const char *reject);
char *arena_strtok(char *s, const char *delim);
void *arena_memchr(const void *s, int c, size_t n);
size_t arena_strnlen(const char *s, size_t max);
int arena_strncmp(const char *a, const char *b, size_t n);
char *arena_strcpy(char *dst, const char *src);
char *arena_strncpy(char *dst, const char *src, size_t n);
char *arena_strcat(char *dst, const char *src);
char *arena_strncat(char *dst, const char *src, size_t n);
void *arena_aligned_alloc(size_t align, size_t n);
void *arena_std_aligned_alloc(size_t align, size_t n);
void *arena_std_calloc(size_t count, size_t size);
int arena_atexit(void (*fn)(void));
int arena_abs(int v);
int arena_atoi(const char *s);
char *arena_getenv(const char *name);
int arena_rand(void);
void arena_srand(unsigned int seed);
/* C2.6 coverage: refusal paths, va_list formatter entry points, errno location, and
 * the process-ending paths (checked in a forked child). Host-only evidence. */
int arena_vsprintf(char *buf, const char *fmt, va_list ap);
int arena_vprintf(const char *fmt, va_list ap);
int arena_vfprintf(struct arena_file *f, const char *fmt, va_list ap);
struct arena_file *arena_fopen(const char *path, const char *mode);
struct arena_file *arena_freopen(const char *path, const char *mode, struct arena_file *f);
int arena_remove(const char *path);
int arena_rename(const char *from, const char *to);
int arena_fgetc(struct arena_file *f);
int arena_getc(struct arena_file *f);
char *arena_fgets(char *buf, int n, struct arena_file *f);
size_t arena_fread(void *p, size_t size, size_t n, struct arena_file *f);
int arena_fclose(struct arena_file *f);
int *__arena_errno_location(void);

/* C2.1 host seams (host/host_main.c). */
extern long arena_host_write_limit;
extern long arena_host_write_fail;
extern long arena_host_write_budget;
extern int arena_host_capture_on;
extern char arena_host_capture[512];
extern size_t arena_host_capture_n;
extern long arena_host_write_calls;
/* stdio objects (src/stdio.c) */
struct arena_file *arena_stdin_object(void);
struct arena_file *arena_stdout_object(void);
struct arena_file *arena_stderr_object(void);
int arena_fputs(const char *s, struct arena_file *f);
int arena_getchar(void);
int arena_ungetc(int c, struct arena_file *f);
size_t arena_fwrite(const void *p, size_t size, size_t n, struct arena_file *f);
int arena_ferror(struct arena_file *f);
int arena_feof(struct arena_file *f);
void arena_clearerr(struct arena_file *f);
int arena_setvbuf(struct arena_file *f, char *buf, int mode, size_t size);
int arena_printf(const char *fmt, ...);

void arena_libc_abort(void) __attribute__((noreturn));
void arena_libc_Exit(int status) __attribute__((noreturn));
void __arena_assert_fail(const char *expr, const char *file, int line) __attribute__((noreturn));

long arena_labs(long v);
long long arena_llabs(long long v);
long arena_atol(const char *s);
long long arena_atoll(const char *s);
div_t arena_div(int a, int b);
ldiv_t arena_ldiv(long a, long b);
lldiv_t arena_lldiv(long long a, long long b);
int arena_strcoll(const char *a, const char *b);
int arena_sprintf(char *buf, const char *fmt, ...);

static int checks;
static int failures;

#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        checks++;                                                              \
        if (!(cond)) {                                                         \
            failures++;                                                        \
            fprintf(stderr, "FAIL libc %s:%d: %s -- ", __FILE__, __LINE__, #cond); \
            fprintf(stderr, __VA_ARGS__);                                      \
            fprintf(stderr, "\n");                                             \
        }                                                                      \
    } while (0)

/* ---- formatter: differential against glibc ------------------------------- */

#define FMT_CASE_INT(FMT, VAL)                                                  \
    do {                                                                        \
        char got[256], want[256];                                               \
        memset(got, 'Z', sizeof got);                                           \
        memset(want, 'Z', sizeof want);                                         \
        int rg = arena_snprintf(got, sizeof got, FMT, (int)(VAL));              \
        int rw = snprintf(want, sizeof want, FMT, (int)(VAL));                  \
        CHECK(rg == rw && strcmp(got, want) == 0, "fmt %s value %lld: got \"%s\" (%d) want \"%s\" (%d)", \
              FMT, (long long)(VAL), got, rg, want, rw);                        \
    } while (0)

#define FMT_CASE_LONG(FMT, VAL)                                                 \
    do {                                                                        \
        char got[256], want[256];                                               \
        memset(got, 'Z', sizeof got);                                           \
        memset(want, 'Z', sizeof want);                                         \
        int rg = arena_snprintf(got, sizeof got, FMT, (long long)(VAL));        \
        int rw = snprintf(want, sizeof want, FMT, (long long)(VAL));            \
        CHECK(rg == rw && strcmp(got, want) == 0, "fmt %s value %lld: got \"%s\" (%d) want \"%s\" (%d)", \
              FMT, (long long)(VAL), got, rg, want, rw);                        \
    } while (0)

static void test_formatter_diff(void) {
    static const int ints[] = {0, 1, -1, 7, -7, 42, 255, -256, 65535, 2147483647, (-2147483647 - 1)};
    for (size_t i = 0; i < sizeof ints / sizeof ints[0]; i++) {
        int v = ints[i];
        FMT_CASE_INT("%d", v);
        FMT_CASE_INT("%i", v);
        FMT_CASE_INT("%5d", v);
        FMT_CASE_INT("%-5d|", v);
        FMT_CASE_INT("%05d", v);
        FMT_CASE_INT("%+d", v);
        FMT_CASE_INT("% d", v);
        FMT_CASE_INT("%.3d", v);
        FMT_CASE_INT("%8.3d", v);
        FMT_CASE_INT("%x", v);
        FMT_CASE_INT("%X", v);
        FMT_CASE_INT("%#x", v);
        FMT_CASE_INT("%#o", v);
        FMT_CASE_INT("%o", v);
        FMT_CASE_INT("%08X", v);
        FMT_CASE_INT("%u", v);
        FMT_CASE_INT("%hhd", v);
        FMT_CASE_INT("%hd", v);
    }
    static const long long longs[] = {0LL, 1LL, -1LL, 9223372036854775807LL, (-9223372036854775807LL - 1LL), 1234567890123LL};
    for (size_t i = 0; i < sizeof longs / sizeof longs[0]; i++) {
        long long v = longs[i];
        FMT_CASE_LONG("%lld", v);
        FMT_CASE_LONG("%20lld", v);
        FMT_CASE_LONG("%-20lld|", v);
        FMT_CASE_LONG("%llx", v);
        FMT_CASE_LONG("%llu", v);
        FMT_CASE_LONG("%llX", v);
    }
    /* strings, precision, width, percent */
    char got[256], want[256];
    const char *strs[] = {"", "a", "hello", "Arena OS"};
    for (size_t i = 0; i < 4; i++) {
        arena_snprintf(got, sizeof got, "[%s]", strs[i]);
        snprintf(want, sizeof want, "[%s]", strs[i]);
        CHECK(strcmp(got, want) == 0, "%%s basic \"%s\"", strs[i]);
        arena_snprintf(got, sizeof got, "[%8s|%-8s|%.3s]", strs[i], strs[i], strs[i]);
        snprintf(want, sizeof want, "[%8s|%-8s|%.3s]", strs[i], strs[i], strs[i]);
        CHECK(strcmp(got, want) == 0, "%%s width/precision \"%s\"", strs[i]);
    }
    arena_snprintf(got, sizeof got, "100%% %c%c", 'o', 'k');
    CHECK(strcmp(got, "100% ok") == 0, "%%%% and %%c: \"%s\"", got);
    int rc = arena_snprintf(got, sizeof got, "%p", (void *)0);
    CHECK(rc == 5 && strcmp(got, "(nil)") == 0, "%%p NULL is \"(nil)\" (glibc form)");
    CHECK(arena_snprintf(got, sizeof got, "%*d|", 6, 42) == 7 && strcmp(got, "    42|") == 0,
          "%%*d width from argument: \"%s\"", got);
    CHECK(arena_snprintf(got, sizeof got, "%-*d|", 6, 42) == 7 && strcmp(got, "42    |") == 0,
          "%%-*d width from argument: \"%s\"", got);
}

static void test_formatter_truncation(void) {
    char small[4];
    memset(small, 'Z', sizeof small);
    int r = arena_snprintf(small, sizeof small, "%d", 123456);
    CHECK(r == 6, "snprintf returns the full length (6), got %d", r);
    CHECK(strcmp(small, "123") == 0, "snprintf truncates and terminates: \"%s\"", small);
    char untouched[8];
    memset(untouched, 'Z', sizeof untouched);
    r = arena_snprintf(untouched, 0, "%s", "abc");
    CHECK(r == 3 && untouched[0] == 'Z', "snprintf with n=0 writes nothing, returns length");
}

/* Refused conversions: -1, errno set, and no bytes written. */
static void expect_refused(const char *fmt, int want_errno, const char *label) {
    char buf[64];
    memset(buf, 'Z', sizeof buf);
    arena_c_set_errno(0);
    int r = arena_snprintf(buf, sizeof buf, fmt, 1.5);
    CHECK(r == -1, "%s refused: return -1, got %d", label, r);
    CHECK(arena_c_get_errno() == want_errno, "%s refused: errno %d, want %d", label, arena_c_get_errno(), want_errno);
    CHECK(buf[0] == 'Z', "%s refused: no partial output (buf[0]=%d)", label, (int)buf[0]);
}

static void test_formatter_refusal(void) {
    expect_refused("%f", ENOTSUP, "%f");
    expect_refused("%e", ENOTSUP, "%e");
    expect_refused("%g", ENOTSUP, "%g");
    expect_refused("%a", ENOTSUP, "%a");
    expect_refused("%F", ENOTSUP, "%F");
    expect_refused("%10.2f", ENOTSUP, "%10.2f");
    expect_refused("%Lf", ENOTSUP, "%Lf");
    /* Unknown conversions are echoed, not refused (C1 contract). */
    {
        char eb[32];
        int er = arena_snprintf(eb, sizeof eb, "a%yb%");
        CHECK(er == 5 && strcmp(eb, "a%yb%") == 0, "unknown and trailing conversions echo: \"%s\" (%d)", eb, er);
    }
    /* A refused conversion AFTER valid text must produce no output at all. */
    char buf[64];
    memset(buf, 'Z', sizeof buf);
    arena_c_set_errno(0);
    int r = arena_snprintf(buf, sizeof buf, "valid text then %f", 1.0);
    CHECK(r == -1 && buf[0] == 'Z', "dry-run refusal leaves the buffer untouched");
    /* %n writes through its argument: refused. */
    int victim = 0;
    arena_c_set_errno(0);
    r = arena_snprintf(buf, sizeof buf, "abc%n", &victim);
    CHECK(r == -1 && victim == 0 && arena_c_get_errno() == EINVAL, "%%n refused without writing");
}

/* ---- integer parsing: differential against glibc -------------------------- */

static void test_strtol_diff(void) {
    static const char *inputs[] = {
        "0", "  42xyz", "-17", "+9", "0x1f", "0X1F", "0755", "0b101", "z", "",
        "  ", "-", "9223372036854775807", "9223372036854775808", "-9223372036854775808",
        "-9223372036854775809", "18446744073709551615", "18446744073709551616",
        "123abc", "  -0x10", "077", "zz", "1e5", "+0x", "0x", "Ab", "-0",
    };
    static const int bases[] = {0, 2, 8, 10, 16, 36};
    for (size_t i = 0; i < sizeof inputs / sizeof inputs[0]; i++) {
        for (size_t b = 0; b < sizeof bases / sizeof bases[0]; b++) {
            const char *s = inputs[i];
            char *ge = NULL, *we = NULL;
            arena_c_set_errno(0);
            long long gv = arena_strtoll(s, &ge, bases[b]);
            int gerr = arena_c_get_errno();
            errno = 0;
            long long wv = strtoll(s, &we, bases[b]);
            int werr = errno;
            CHECK(gv == wv, "strtoll(\"%s\", base %d): got %lld want %lld", s, bases[b], gv, wv);
            CHECK((ge - s) == (we - s), "strtoll(\"%s\", base %d): end offset %ld want %ld", s, bases[b],
                  (long)(ge - s), (long)(we - s));
            if (werr == ERANGE) {
                CHECK(gerr == ERANGE, "strtoll(\"%s\"): ERANGE expected", s);
            }
            arena_c_set_errno(0);
            unsigned long long gu = arena_strtoull(s, NULL, bases[b]);
            int guerr = arena_c_get_errno();
            errno = 0;
            unsigned long long wu = strtoull(s, NULL, bases[b]);
            int wuerr = errno;
            CHECK(gu == wu, "strtoull(\"%s\", base %d): got %llu want %llu", s, bases[b], gu, wu);
            if (wuerr == ERANGE) {
                CHECK(guerr == ERANGE, "strtoull(\"%s\"): ERANGE expected", s);
            }
        }
    }
    /* long/unsigned long forms on LP64 match the long long forms */
    CHECK(arena_strtol("12345", NULL, 10) == 12345L, "strtol basic");
    CHECK(arena_strtoul("4294967296", NULL, 10) == 4294967296UL, "strtoul above 32 bits");
    CHECK(arena_atoi("  -99") == -99, "atoi with leading space and sign");
    CHECK(arena_abs(-5) == 5 && arena_abs(5) == 5, "abs");
}

/* ---- string functions: differential against glibc ------------------------- */

static void test_string_diff(void) {
    const char *hay = "the quick brown fox jumps over the lazy dog";
    CHECK(arena_strchr(hay, 'q') == strchr(hay, 'q'), "strchr found");
    CHECK(arena_strchr(hay, 'Q') == NULL && strchr(hay, 'Q') == NULL, "strchr absent");
    CHECK(arena_strchr(hay, '\0') == strchr(hay, '\0'), "strchr finds the terminator");
    CHECK(arena_strrchr(hay, 'o') == strrchr(hay, 'o'), "strrchr last occurrence");
    CHECK(arena_strstr(hay, "lazy") == strstr(hay, "lazy"), "strstr found");
    CHECK(arena_strstr(hay, "cat") == NULL, "strstr absent");
    CHECK(arena_strstr(hay, "") == hay, "strstr empty needle");
    CHECK(arena_strspn(hay, "the ") == strspn(hay, "the "), "strspn");
    CHECK(arena_strcspn(hay, "xyz") == strcspn(hay, "xyz"), "strcspn");
    CHECK(arena_strpbrk(hay, "zl") == strpbrk(hay, "zl"), "strpbrk");
    CHECK(arena_memchr(hay, 'j', 40) == memchr(hay, 'j', 40), "memchr");
    CHECK(arena_memchr(hay, 'j', 10) == NULL, "memchr bounded");
    CHECK(arena_strnlen("abcdef", 3) == 3 && arena_strnlen("ab", 9) == 2, "strnlen");
    CHECK(arena_strncmp("abcd", "abce", 3) == 0 && arena_strncmp("abcd", "abce", 4) < 0,
          "strncmp bound");

    char a[32], b[32];
    memset(a, 'Z', sizeof a);
    memset(b, 'Z', sizeof b);
    arena_strncpy(a, "hi", 6);
    strncpy(b, "hi", 6);
    CHECK(memcmp(a, b, 6) == 0, "strncpy pads with NULs like glibc");
    CHECK(a[6] == 'Z', "strncpy does not write past n");
    arena_strcpy(a, "abc");
    arena_strcat(a, "def");
    CHECK(strcmp(a, "abcdef") == 0, "strcat");
    arena_strncat(a, "ghijk", 2);
    CHECK(strcmp(a, "abcdefgh") == 0, "strncat");

    /* strtok sequence equals glibc's */
    char t1[64], t2[64];
    strcpy(t1, "  alpha,beta;;gamma delta ");
    strcpy(t2, t1);
    char *g = strtok(t1, " ,;");
    char *w = arena_strtok(t2, " ,;");
    int same = 1;
    while (g != NULL || w != NULL) {
        if (g == NULL || w == NULL || strcmp(g, w) != 0) {
            same = 0;
            break;
        }
        g = strtok(NULL, " ,;");
        w = arena_strtok(NULL, " ,;");
    }
    CHECK(same, "strtok token sequence equals glibc");
}

/* ---- qsort / bsearch ------------------------------------------------------- */

static int cmp_int(const void *a, const void *b) {
    int x = *(const int *)a, y = *(const int *)b;
    return (x > y) - (x < y);
}

static void test_qsort(void) {
    int v[257], w[257];
    for (int round = 0; round < 40; round++) {
        size_t n = (size_t)(round * 7) % 257;
        for (size_t i = 0; i < n; i++) {
            v[i] = (int)((i * 2654435761u + (size_t)round * 40503u) % 1000u) - 500;
            w[i] = v[i];
        }
        arena_qsort(v, n, sizeof v[0], cmp_int);
        qsort(w, n, sizeof w[0], cmp_int);
        CHECK(memcmp(v, w, n * sizeof v[0]) == 0, "qsort n=%zu matches glibc", n);
        if (n > 0) {
            int key = v[n / 2];
            int *hit = arena_bsearch(&key, v, n, sizeof v[0], cmp_int);
            CHECK(hit != NULL && *hit == key, "bsearch finds an element");
        }
    }
    int missing = 100000;
    int sorted[4] = {1, 2, 3, 4};
    CHECK(arena_bsearch(&missing, sorted, 4, sizeof sorted[0], cmp_int) == NULL, "bsearch absent");
}

/* ---- aligned allocation and invalid-pointer refusal ----------------------- */

static void test_aligned_alloc(void) {
    struct arena_heap_stats s0, s1, s2;
    arena_heap_stats(&s0);
    static const size_t aligns[] = {16, 32, 64, 128, 256, 1024, 4096, 16384, 65536};
    for (size_t i = 0; i < sizeof aligns / sizeof aligns[0]; i++) {
        size_t al = aligns[i];
        size_t n = 100 + i * 37;
        unsigned char *p = arena_aligned_alloc(al, n);
        CHECK(p != NULL, "aligned_alloc(%zu, %zu) returned NULL", al, n);
        if (p == NULL) {
            continue;
        }
        CHECK(((uintptr_t)p % al) == 0, "aligned_alloc(%zu) pointer %p is misaligned", al, (void *)p);
        memset(p, (int)(i + 1), n);
        unsigned char *q = arena_realloc(p, n * 3);
        CHECK(q != NULL, "realloc of an aligned block");
        if (q != NULL) {
            CHECK(q[0] == (unsigned char)(i + 1) && q[n - 1] == (unsigned char)(i + 1),
                  "realloc preserved the aligned block contents");
            arena_free(q);
        }
    }
    CHECK(arena_aligned_alloc(3, 10) == NULL, "non-power-of-two alignment refused");
    CHECK(arena_aligned_alloc(0, 10) == NULL, "zero alignment refused");
    CHECK(arena_aligned_alloc(131072, 10) == NULL, "alignment above the limit refused");
    CHECK(arena_aligned_alloc(64, (size_t)-1) == NULL, "huge size refused without overflow");

    /* standard wrappers set errno on refusal (EINVAL for a bad alignment, ENOMEM otherwise) */
    arena_c_set_errno(0);
    CHECK(arena_std_aligned_alloc(3, 8) == NULL && arena_c_get_errno() == EINVAL,
          "aligned_alloc(3) sets EINVAL (errno %d)", arena_c_get_errno());
    arena_c_set_errno(0);
    CHECK(arena_std_calloc((size_t)-1, 2) == NULL && arena_c_get_errno() == ENOMEM,
          "calloc overflow sets ENOMEM (errno %d)", arena_c_get_errno());

    /* invalid frees are refused and counted, never reach memory outside the heap */
    arena_heap_stats(&s1);
    unsigned char *ok = arena_aligned_alloc(256, 64);
    CHECK(ok != NULL, "aligned_alloc for invalid-free tests");
    if (ok != NULL) {
        arena_free(ok + 1);      /* not a block start */
        arena_free(ok + 256 - 16); /* inside the pad, not the user pointer */
        arena_heap_stats(&s2);
        CHECK(s2.bad_frees >= s1.bad_frees + 2, "invalid frees counted as bad (%llu -> %llu)",
              (unsigned long long)s1.bad_frees, (unsigned long long)s2.bad_frees);
        arena_free(ok);
        arena_free(ok); /* double free: refused */
    }
    arena_heap_stats(&s2);
    CHECK(s2.live_blocks == s0.live_blocks, "live blocks return to baseline (%llu vs %llu)",
          (unsigned long long)s2.live_blocks, (unsigned long long)s0.live_blocks);
}

/* ---- env, misc ------------------------------------------------------------- */

static void test_env_and_rand(void) {
    CHECK(arena_getenv("PATH") == NULL, "getenv without a startup record returns NULL");
    arena_srand(1u);
    int a = arena_rand();
    arena_srand(1u);
    CHECK(arena_rand() == a && a >= 0 && a <= 32767, "rand is deterministic per seed and in range");
}

/* ---- errno mapping -------------------------------------------------------- */

static void test_errno_map(void) {
    CHECK(arena_c_errno_for(ARENA_E_NOMEM) == ENOMEM, "NOMEM maps to ENOMEM");
    CHECK(arena_c_errno_for(ARENA_E_WOULD_BLOCK) == EAGAIN, "WOULD_BLOCK maps to EAGAIN");
    CHECK(arena_c_errno_for(ARENA_E_BROKEN_PIPE) == EPIPE, "BROKEN_PIPE maps to EPIPE");
    CHECK(arena_c_errno_for(ARENA_E_NO_STREAMS) == EBADF, "NO_STREAMS maps to EBADF");
    CHECK(arena_c_errno_for(ARENA_E_NO_CAP) == EPERM, "NO_CAP maps to EPERM");
    CHECK(arena_c_errno_for(ARENA_E_UNSUPPORTED) == ENOTSUP, "UNSUPPORTED maps to ENOTSUP");
    CHECK(arena_c_errno_for(-7) == EAGAIN, "QUOTA maps to EAGAIN");
    CHECK(arena_c_errno_for(-99999) == EIO, "unknown status maps to EIO");
    CHECK(strcmp(strerror(ENOTSUP), strerror(ENOTSUP)) == 0, "strerror stable");
}


/* ---- C2.1.3 stdio contract ------------------------------------------------- */

/* fwrite returns complete elements actually written, on every path. */
static void test_fwrite_contract(void) {
    static const char data[] = "0123456789ABCDEF";
    arena_host_capture_on = 1;
    arena_host_capture_n = 0;
    arena_host_write_limit = 0;
    arena_host_write_fail = 0;
    arena_host_write_budget = -1;
    arena_clearerr(arena_stdout_object());

    CHECK(arena_fwrite(data, 1, 4, arena_stdout_object()) == 4 && arena_host_capture_n == 4 &&
              memcmp(arena_host_capture, "0123", 4) == 0,
          "fwrite full: 4 one-byte items, bytes in order");

    arena_host_write_limit = 2; /* short writes: the loop retries until the element set is done */
    arena_host_capture_n = 0;
    CHECK(arena_fwrite(data, 4, 3, arena_stdout_object()) == 3 && arena_host_capture_n == 12 &&
              memcmp(arena_host_capture, data, 12) == 0,
          "fwrite short writes: 3 items reported, 12 bytes intact");
    arena_host_write_limit = 0;

    /* Incomplete element: budget 6 of 3 x 4-byte items -> item 1 complete, item 2 partial. */
    arena_host_capture_n = 0;
    arena_host_write_budget = 6; /* budget exhausts mid-element; fail seam stays 0 (fallback EPIPE) */
    arena_host_write_fail = 0;
    arena_c_set_errno(0);
    size_t got = arena_fwrite(data, 4, 3, arena_stdout_object());
    CHECK(got == 1, "fwrite incomplete element: reports 1 complete item (got %zu)", got);
    CHECK(arena_host_capture_n == 6, "fwrite incomplete element: exactly the 6 accepted bytes written");
    CHECK(arena_ferror(arena_stdout_object()) != 0, "fwrite incomplete element: sticky error flag set");
    CHECK(arena_c_get_errno() == EPIPE, "fwrite incomplete element: errno EPIPE (got %d)", arena_c_get_errno());
    arena_host_write_budget = -1;

    /* The error flag is sticky until cleared. */
    CHECK(arena_ferror(arena_stdout_object()) != 0, "ferror stays set until clearerr");
    arena_clearerr(arena_stdout_object());
    CHECK(arena_ferror(arena_stdout_object()) == 0, "clearerr clears ferror");

    /* Failing from the first byte: zero complete items. */
    arena_host_write_budget = 0;
    arena_host_write_fail = 0;
    CHECK(arena_fwrite(data, 1, 4, arena_stdout_object()) == 0 && arena_ferror(arena_stdout_object()) != 0,
          "fwrite failing: 0 items, error flag");
    arena_host_write_budget = -1;
    arena_host_write_fail = 0;
    arena_clearerr(arena_stdout_object());

    /* Zero size or zero count: 0, no error, no write. */
    arena_host_capture_n = 0;
    CHECK(arena_fwrite(data, 0, 5, arena_stdout_object()) == 0 && arena_ferror(arena_stdout_object()) == 0 && arena_host_capture_n == 0,
          "fwrite zero size: 0, no error, no write");
    CHECK(arena_fwrite(data, 5, 0, arena_stdout_object()) == 0 && arena_ferror(arena_stdout_object()) == 0 && arena_host_capture_n == 0,
          "fwrite zero count: 0, no error, no write");

    /* Overflow of size * n: refused before any write, EOVERFLOW, not an I/O error. */
    arena_c_set_errno(0);
    CHECK(arena_fwrite(data, (size_t)-1 / 2 + 1, 2, arena_stdout_object()) == 0 && arena_c_get_errno() == 75 &&
              arena_host_capture_n == 0 && arena_ferror(arena_stdout_object()) == 0,
          "fwrite size*n overflow: 0, EOVERFLOW, no write, no I/O error");

    /* Not writable: stdin. */
    arena_c_set_errno(0);
    CHECK(arena_fwrite(data, 1, 4, arena_stdin_object()) == 0 && arena_c_get_errno() == EBADF,
          "fwrite on stdin refused (EBADF)");

    arena_host_capture_on = 0;
    arena_host_capture_n = 0;
}

/* setvbuf implements _IONBF only. Buffered modes are refused, never ignored. */
static void test_setvbuf_policy(void) {
    static char buf[128];
    /* _IOFBF 0, _IOLBF 1, _IONBF 2 (include/libc/stdio.h). */
    CHECK(arena_setvbuf(arena_stdout_object(), NULL, 2, 0) == 0, "setvbuf _IONBF accepted (unbuffered)");
    CHECK(arena_setvbuf(arena_stdout_object(), buf, 2, sizeof buf) == 0, "setvbuf _IONBF with a buffer: buffer ignored, accepted");
    arena_c_set_errno(0);
    CHECK(arena_setvbuf(arena_stdout_object(), buf, 0, sizeof buf) == -1 && arena_c_get_errno() == EINVAL,
          "setvbuf _IOFBF refused with EINVAL");
    arena_c_set_errno(0);
    CHECK(arena_setvbuf(arena_stdout_object(), buf, 1, sizeof buf) == -1 && arena_c_get_errno() == EINVAL,
          "setvbuf _IOLBF refused with EINVAL");
    arena_c_set_errno(0);
    CHECK(arena_setvbuf(arena_stdout_object(), NULL, 9, 0) == -1 && arena_c_get_errno() == EINVAL,
          "setvbuf unknown mode refused with EINVAL");
    arena_c_set_errno(0);
    CHECK(arena_setvbuf(NULL, NULL, 2, 0) == -1 && arena_c_get_errno() == EBADF,
          "setvbuf on NULL stream refused with EBADF");
}

/* Input: a read error is not end of file; pushback is one byte; no ambient read. */
static void test_input_contract(void) {
    char line[16];
    arena_clearerr(arena_stdin_object());
    /* No stream grant: the read fails. fgets returns NULL and sets ferror (not feof). */
    CHECK(arena_fgets(line, sizeof line, arena_stdin_object()) == NULL, "fgets read error returns NULL");
    CHECK(arena_ferror(arena_stdin_object()) != 0 && arena_feof(arena_stdin_object()) == 0, "fgets read error sets ferror, not feof");
    arena_clearerr(arena_stdin_object());

    /* ungetc pushback is deterministic: getchar returns it without touching a stream. */
    CHECK(arena_ungetc('Q', arena_stdin_object()) == 'Q', "ungetc accepts one byte");
    CHECK(arena_ungetc('R', arena_stdin_object()) == -1, "a second ungetc is refused (one byte of pushback)");
    CHECK(arena_getchar() == 'Q', "getchar returns the pushed-back byte, no stream read");
    CHECK(arena_ungetc(-1, arena_stdin_object()) == -1, "ungetc(EOF) refused");
    CHECK(arena_ungetc(256, arena_stdin_object()) == -1, "ungetc out-of-range refused");
    arena_clearerr(arena_stdin_object());
}

/* ---- C2.1.3 formatter bounds (INT_MAX) ----------------------------------- */

static void test_formatter_bounds(void) {
    char b[16];
    memset(b, '#', sizeof b);
    arena_c_set_errno(0);
    CHECK(arena_snprintf(b, sizeof b, "%*d", -2147483647 - 1, 1) == -1 && arena_c_get_errno() == 75,
          "* width INT_MIN refused (EOVERFLOW)");
    arena_c_set_errno(0);
    CHECK(arena_snprintf(b, sizeof b, "%2147483648d", 1) == -1 && arena_c_get_errno() == 75,
          "width 2^31 refused (EOVERFLOW)");
    arena_c_set_errno(0);
    CHECK(arena_snprintf(b, sizeof b, "%.2147483648d", 1) == -1 && arena_c_get_errno() == 75,
          "precision 2^31 refused (EOVERFLOW)");
    arena_c_set_errno(0);
    CHECK(arena_snprintf(b, sizeof b, "%99999999999999999999d", 1) == -1 && arena_c_get_errno() == 75,
          "twenty-digit width refused (EOVERFLOW)");
    CHECK(b[0] == '#' && b[sizeof b - 1] == '#', "refused snprintf writes nothing");

    /* Width exactly INT_MAX: an exact count, bounded output, no per-byte loop. */
    int n = arena_snprintf(b, sizeof b, "%2147483647d", 1);
    CHECK(n == 2147483647, "width INT_MAX: exact count (got %d)", n);
    CHECK(b[0] == ' ' && b[sizeof b - 1] == '\0', "width INT_MAX: buffer padded then terminated");

    /* The total crossing INT_MAX is refused, even though each part is in range. */
    memset(b, '#', sizeof b);
    arena_c_set_errno(0);
    CHECK(arena_snprintf(b, sizeof b, "xxx%2147483647d", 1) == -1 && arena_c_get_errno() == 75,
          "total above INT_MAX refused (EOVERFLOW)");
    CHECK(b[0] == '#' && b[sizeof b - 1] == '#', "total above INT_MAX: buffer untouched");

    /* A refused printf writes nothing to the stream. */
    arena_host_capture_on = 1;
    arena_host_capture_n = 0;
    CHECK(arena_printf("never%2147483648d\n", 1) == -1 && arena_host_capture_n == 0,
          "refused printf: no bytes reach the stream");
    arena_host_capture_on = 0;
}

/* ---- C2.1.4 atexit concurrency (forked children; the parent registry stays clean) */

#define AH(i) static void ah_##i(void) { (void)__atomic_fetch_add(&ah_hits[i], 1u, __ATOMIC_ACQ_REL); }
static unsigned ah_hits[40];
static int reg_ok[40];
static unsigned reenter_hits;
static void reenter_handler(void);
AH(0) AH(1) AH(2) AH(3) AH(4) AH(5) AH(6) AH(7) AH(8) AH(9)
AH(10) AH(11) AH(12) AH(13) AH(14) AH(15) AH(16) AH(17) AH(18) AH(19)
AH(20) AH(21) AH(22) AH(23) AH(24) AH(25) AH(26) AH(27) AH(28) AH(29)
AH(30) AH(31) AH(32) AH(33) AH(34) AH(35) AH(36) AH(37) AH(38) AH(39)
#undef AH
static void (*const ah_fns[40])(void) = {
    ah_0, ah_1, ah_2, ah_3, ah_4, ah_5, ah_6, ah_7, ah_8, ah_9,
    ah_10, ah_11, ah_12, ah_13, ah_14, ah_15, ah_16, ah_17, ah_18, ah_19,
    ah_20, ah_21, ah_22, ah_23, ah_24, ah_25, ah_26, ah_27, ah_28, ah_29,
    ah_30, ah_31, ah_32, ah_33, ah_34, ah_35, ah_36, ah_37, ah_38, ah_39};

static void reenter_handler(void) {
    reenter_hits++;
    arena_c_run_exit_handlers(); /* same thread, inside a handler: must return at once */
}

static void *registrar(void *arg) {
    long k = (long)arg;
    for (int j = 0; j < 10; j++) {
        int idx = (int)(k * 10 + j); /* 4 x 10 = 40 distinct attempts, capacity 32 */
        reg_ok[idx] = arena_atexit(ah_fns[idx]) == 0;
    }
    return NULL;
}

static void *runner(void *arg) {
    (void)arg;
    arena_c_run_exit_handlers();
    return NULL;
}

/* Race: 4 registrars against a runner. Exactly 32 succeed; no handler runs twice;
 * a handler never runs for a refused registration; no unpublished slot is called. */
static int child_race(void) {
    pthread_t reg[4], run;
    for (long k = 0; k < 4; k++) {
        if (pthread_create(&reg[k], NULL, registrar, (void *)k) != 0) return 10;
    }
    if (pthread_create(&run, NULL, runner, NULL) != 0) return 11;
    for (int k = 0; k < 4; k++) (void)pthread_join(reg[k], NULL);
    (void)pthread_join(run, NULL);
    int ok = 0;
    for (int i = 0; i < 40; i++) ok += reg_ok[i];
    if (ok != 32) return 12;
    arena_c_run_exit_handlers(); /* no-op or the first run: either way once */
    for (int i = 0; i < 40; i++) {
        if (ah_hits[i] > 1u) return 13;                 /* at most once */
        if (!reg_ok[i] && ah_hits[i] != 0u) return 14; /* refused: never runs */
    }
    return 0;
}

/* Ordered: every registration that returned before exit began runs exactly once,
 * re-entry from a handler is refused, and a second exit run does nothing. */
static int child_ordered(void) {
    for (int i = 0; i < 31; i++) reg_ok[i] = arena_atexit(ah_fns[i]) == 0;
    int reenter_ok = arena_atexit(reenter_handler) == 0;
    for (int i = 0; i < 31; i++) if (!reg_ok[i]) return 20;
    if (!reenter_ok) return 21;
    if (arena_atexit(ah_fns[39]) != -1) return 22; /* registry full: 32 of 32 */
    arena_c_run_exit_handlers();
    for (int i = 0; i < 31; i++) if (ah_hits[i] != 1u) return 23;
    if (reenter_hits != 1u) return 24;
    arena_c_run_exit_handlers();
    for (int i = 0; i < 31; i++) if (ah_hits[i] != 1u) return 25;
    if (reenter_hits != 1u) return 26;
    return 0;
}

static int run_child(int (*fn)(void)) {
    pid_t pid = fork();
    if (pid == 0) {
        _exit(fn());
    }
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st)) {
        return -1;
    }
    return WEXITSTATUS(st);
}

static void test_atexit_concurrency(void) {
    int race = run_child(child_race);
    CHECK(race == 0, "atexit race: exactly 32 registrations, no double run, no call of a refused slot (child %d)", race);
    int ordered = run_child(child_ordered);
    CHECK(ordered == 0, "atexit ordered: registered handlers run once, re-entry refused, no re-run (child %d)", ordered);
}

/* ---- C2.1.5 notification selection (pure policy, startup_query.c) ----------- */

static arena_cap_desc_t cap_of(uint16_t slot, uint8_t role, uint8_t kind, uint32_t rights) {
    arena_cap_desc_t d;
    d.slot = slot;
    d.role = role;
    d.kind = kind;
    d.rights = rights;
    return d;
}

static void test_notification_selection(void) {
    const uint32_t RW = ARENA_RIGHT_READ | ARENA_RIGHT_WRITE;
    const uint32_t RO = ARENA_RIGHT_READ;
    arena_cap_desc_t c[6];
    const unsigned NONE = ARENA_ARST_NONE;

    /* Headless: no caps at all. */
    CHECK(arena_startup_pick_notification(NULL, 0) == NONE, "notification: headless (no table) -> NONE");
    CHECK(arena_startup_pick_notification(c, 0) == NONE, "notification: headless (empty table) -> NONE");

    /* Missing: a sync domain and a window surface, no notification. */
    c[0] = cap_of(1, ARENA_ARST_ROLE_SYNC_DOMAIN, ARENA_CAP_KIND_SYNC_DOMAIN, RW);
    c[1] = cap_of(2, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_SHARED_REGION, RW);
    CHECK(arena_startup_pick_notification(c, 2) == NONE, "notification: missing -> NONE");

    /* One valid. */
    c[2] = cap_of(3, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, RW);
    CHECK(arena_startup_pick_notification(c, 3) == 2, "notification: one valid -> its index");

    /* Two equal: ambiguous, nothing is guessed. */
    c[3] = cap_of(4, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, RW);
    CHECK(arena_startup_pick_notification(c, 4) == NONE, "notification: two equal candidates -> NONE (no guess)");

    /* Wrong rights: a read-only notification is not a candidate, so the RW one is the sole one. */
    arena_cap_desc_t w[3];
    w[0] = cap_of(5, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, RO);
    w[1] = cap_of(6, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, RW);
    w[2] = cap_of(7, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, RO);
    CHECK(arena_startup_pick_notification(w, 3) == 1, "notification: read-only ignored, sole RW chosen");
    CHECK(arena_startup_pick_notification(w, 1) == NONE, "notification: only a read-only one -> NONE");

    /* Wrong kind: a badged endpoint with RW and role OTHER (a graphical service) is not a notification. */
    w[0] = cap_of(8, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_BADGED_ENDPOINT, RW);
    CHECK(arena_startup_pick_notification(w, 1) == NONE, "notification: badged endpoint is not a notification");

    /* Wrong role: a notification that holds the stream-wake role is not an Other candidate. */
    w[0] = cap_of(9, ARENA_ARST_ROLE_STREAM_WAKE, ARENA_CAP_KIND_NOTIFICATION, RW);
    CHECK(arena_startup_pick_notification(w, 1) == NONE, "notification: stream-wake role is not a candidate");

    /* Malformed: a kind outside the table, or empty rights. */
    w[0] = cap_of(10, ARENA_ARST_ROLE_OTHER, 0, RW);
    CHECK(arena_startup_pick_notification(w, 1) == NONE, "notification: malformed kind 0 is not a candidate");
    w[0] = cap_of(11, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, 0);
    CHECK(arena_startup_pick_notification(w, 1) == NONE, "notification: zero rights is not a candidate");

    /* Graphical: a desktop endpoint, a surface, and one notification -> the notification. */
    arena_cap_desc_t g[3];
    g[0] = cap_of(12, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_BADGED_ENDPOINT, RW);
    g[1] = cap_of(13, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_SHARED_REGION, RW);
    g[2] = cap_of(14, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, RW);
    CHECK(arena_startup_pick_notification(g, 3) == 2, "notification: graphical grants do not confuse the sole notification");
    g[1] = cap_of(13, ARENA_ARST_ROLE_OTHER, ARENA_CAP_KIND_NOTIFICATION, RW);
    CHECK(arena_startup_pick_notification(g, 3) == NONE, "notification: two notifications among graphical grants -> NONE");
}

/* ---- atexit: LAST, it alters process-wide exit state ----------------------- */

static int order[4];
static int order_n;
static void h1(void) { order[order_n++] = 1; }
static void h2(void) { order[order_n++] = 2; }
static void h3(void) { order[order_n++] = 3; }

static void test_atexit(void) {
    CHECK(arena_atexit(h1) == 0 && arena_atexit(h2) == 0 && arena_atexit(h3) == 0,
          "three atexit registrations");
    arena_c_run_exit_handlers();
    CHECK(order_n == 3 && order[0] == 3 && order[1] == 2 && order[2] == 1,
          "handlers run in reverse registration order (%d items)", order_n);
    arena_c_run_exit_handlers();
    CHECK(order_n == 3, "handlers are not re-run");
}


/* C2.6 coverage additions: pure functions that had no test reference. Glibc is the
 * differential reference where the semantics are exact. */
static void test_pure_additions(void) {
    CHECK(arena_labs(-5L) == 5L && arena_llabs(-9LL) == 9LL && arena_labs(0L) == 0L, "labs/llabs");
    CHECK(arena_atol(" -123xyz") == -123L && arena_atoll("99999999999") == 99999999999LL,
          "atol/atoll");
    CHECK(arena_atol(" -123xyz") == atol(" -123xyz") && arena_atoll("99999999999") == atoll("99999999999"),
          "atol/atoll match glibc");
    div_t d = arena_div(-7, 2);
    CHECK(d.quot == -3 && d.rem == -1, "div(-7,2)");
    ldiv_t ld = arena_ldiv(7L, -2L);
    CHECK(ld.quot == -3L && ld.rem == 1L, "ldiv(7,-2)");
    lldiv_t lld = arena_lldiv(-9LL, 4LL);
    CHECK(lld.quot == -2LL && lld.rem == -1LL, "lldiv(-9,4)");
    CHECK(arena_strcoll("abc", "abd") < 0 && arena_strcoll("x", "x") == 0, "strcoll in the C locale");
    char a[64], b[64];
    int na = arena_snprintf(a, sizeof a, "%d|%s|%x", -42, "ok", 255u);
    int nb = arena_sprintf(b, "%d|%s|%x", -42, "ok", 255u);
    CHECK(na == (int)strlen("-42|ok|ff") && nb == na && strcmp(a, "-42|ok|ff") == 0 && strcmp(b, a) == 0,
          "sprintf matches snprintf (%d)", na);
    char c[16], w[16];
    /* volatile source pointer: the length is not a compile-time constant */
    const char *volatile src19 = "0123456789abcdefXYZ";
    int nc = arena_snprintf(c, sizeof c, "%s", src19);
    int nw = snprintf(w, sizeof w, "%s", src19);
    CHECK(nc == nw && nc == 19 && strcmp(c, w) == 0, "snprintf truncation reports %d (glibc %d)", nc, nw);
    arena_srand(1u);
    int r1 = arena_rand();
    arena_srand(1u);
    CHECK(arena_rand() == r1, "srand(1) is repeatable");
}


/* C2.4: count-only discovery over a verified record (synthetic here; the guest runs
 * exercise the real ARST path). */
static void test_startup_count(void) {
    arena_startup_t s;
    memset(&s, 0, sizeof s);
    CHECK(arena_startup_count_kind(NULL, ARENA_CAP_KIND_SHARED_REGION, 0) == -1, "count: NULL record");
    CHECK(arena_startup_count_kind(&s, ARENA_CAP_KIND_SHARED_REGION, 0) == -1, "count: no record present");
    s.present = 1;
    s.cap_count = 3;
    s.caps[0].slot = 1; s.caps[0].kind = ARENA_CAP_KIND_SHARED_REGION; s.caps[0].rights = ARENA_RIGHT_READ | ARENA_RIGHT_WRITE;
    s.caps[1].slot = 2; s.caps[1].kind = ARENA_CAP_KIND_NOTIFICATION; s.caps[1].rights = ARENA_RIGHT_WRITE;
    s.caps[2].slot = 3; s.caps[2].kind = ARENA_CAP_KIND_SHARED_REGION; s.caps[2].rights = ARENA_RIGHT_READ;
    CHECK(arena_startup_count_kind(&s, ARENA_CAP_KIND_SHARED_REGION, 0) == 2, "count: two regions");
    CHECK(arena_startup_count_kind(&s, ARENA_CAP_KIND_SHARED_REGION, ARENA_RIGHT_WRITE) == 1,
          "count: rights mask requires WRITE");
    CHECK(arena_startup_count_kind(&s, ARENA_CAP_KIND_BADGED_ENDPOINT, 0) == 0, "count: absent kind is 0");
    s.cap_count = ARENA_ARST_CAPABILITY_MAX + 1u;
    CHECK(arena_startup_count_kind(&s, ARENA_CAP_KIND_SHARED_REGION, 0) == -1, "count: oversized table refused");
}


static int via_vsnprintf(char *b, size_t n, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vsnprintf(b, n, fmt, ap);
    va_end(ap);
    return r;
}

static int via_vsprintf(char *b, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vsprintf(b, fmt, ap);
    va_end(ap);
    return r;
}

static int via_vprintf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vprintf(fmt, ap);
    va_end(ap);
    return r;
}

static int via_vfprintf(struct arena_file *f, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = arena_vfprintf(f, fmt, ap);
    va_end(ap);
    return r;
}

static void child_abort(void) { arena_libc_abort(); }
static void child_exit7(void) { arena_libc_Exit(7); }
static void child_assert(void) { __arena_assert_fail("x", "t.c", 1); }

/* Runs fn in a forked child and returns its exit status, or -1 if it did not exit. */
static int child_status(void (*fn)(void)) {
    fflush(NULL);
    pid_t pid = fork();
    if (pid == 0) {
        fn();
        _exit(99); /* not reached: fn must end the child itself */
    }
    int st = 0;
    if (pid < 0 || waitpid(pid, &st, 0) != pid) {
        return -1;
    }
    return WIFEXITED(st) ? WEXITSTATUS(st) : -1;
}

static void test_refusals_and_exit_paths(void) {
    char a[32], b[32];
    CHECK(via_vsnprintf(a, sizeof a, "%d-%s", 42, "x") == 4 && strcmp(a, "42-x") == 0,
          "vsnprintf through a va_list");
    CHECK(via_vsprintf(b, "%x", 255u) == 2 && strcmp(b, "ff") == 0, "vsprintf through a va_list");
    CHECK(via_vprintf("vp:%d\n", 7) == 5, "vprintf returns the byte count");
    CHECK(via_vfprintf(arena_stdout_object(), "vf:%d\n", 8) == 5, "vfprintf returns the byte count");

    arena_c_set_errno(0);
    *__arena_errno_location() = EPERM;
    CHECK(arena_c_get_errno() == EPERM, "__arena_errno_location is the errno value");
    *__arena_errno_location() = 0;

    arena_c_set_errno(0);
    CHECK(arena_fopen("/no/such", "r") == NULL && arena_c_get_errno() == ENOSYS,
          "fopen refused with ENOSYS (no file authority)");
    arena_c_set_errno(0);
    CHECK(arena_freopen("/no/such", "r", arena_stdout_object()) == NULL && arena_c_get_errno() == ENOSYS,
          "freopen refused with ENOSYS");
    arena_c_set_errno(0);
    CHECK(arena_remove("/no/such") == -1 && arena_c_get_errno() == ENOSYS, "remove refused with ENOSYS");
    arena_c_set_errno(0);
    CHECK(arena_rename("/a", "/b") == -1 && arena_c_get_errno() == ENOSYS, "rename refused with ENOSYS");

    /* NULL streams are rejected with EBADF; the read paths never block here. */
    arena_c_set_errno(0);
    CHECK(arena_fgetc(NULL) == EOF && arena_c_get_errno() == EBADF, "fgetc(NULL) is EOF with EBADF");
    arena_c_set_errno(0);
    CHECK(arena_fgets(a, sizeof a, NULL) == NULL && arena_c_get_errno() == EBADF, "fgets(NULL) is NULL");
    arena_c_set_errno(0);
    CHECK(arena_fread(a, 1, sizeof a, NULL) == 0 && arena_c_get_errno() == EBADF, "fread(NULL) is 0");
    arena_c_set_errno(0);
    CHECK(arena_fclose(NULL) == -1 && arena_c_get_errno() == EBADF, "fclose(NULL) fails with EBADF");
    arena_c_set_errno(0);
    CHECK(arena_getc(NULL) == EOF && arena_c_get_errno() == EBADF, "getc(NULL) is EOF with EBADF");

    /* Process-ending paths, observed from a child. */
    CHECK(child_status(child_abort) == 134, "abort ends with status 134");
    CHECK(child_status(child_exit7) == 7, "_Exit ends with the given status");
    CHECK(child_status(child_assert) == 134, "a failed assertion ends with status 134");
}

int libc_host_checks;

/* A closed stream: writes fail with EBADF and never reach the stream. */
static void test_closed_stream(void) {
    arena_fclose(arena_stderr_object());
    arena_c_set_errno(0);
    CHECK(arena_fwrite("x", 1, 1, arena_stderr_object()) == 0 && arena_c_get_errno() == EBADF,
          "closed stream: fwrite returns 0 with EBADF");
    arena_c_set_errno(0);
    CHECK(arena_fputs("x", arena_stderr_object()) == -1, "closed stream: fputs refused");
}

int libc_host_tests(void) {
    checks = 0;
    failures = 0;
    test_formatter_diff();
    test_formatter_truncation();
    test_formatter_refusal();
    test_strtol_diff();
    test_string_diff();
    test_qsort();
    test_aligned_alloc();
    test_env_and_rand();
    test_errno_map();
    test_pure_additions();
    test_startup_count();
    test_refusals_and_exit_paths();
    test_fwrite_contract();
    test_setvbuf_policy();
    test_input_contract();
    test_formatter_bounds();
    test_notification_selection();
    test_atexit_concurrency(); /* forks: must run before the parent's atexit state is used */
    test_atexit();
    test_closed_stream(); /* LAST: closes stderr for the rest of the process */
    libc_host_checks = checks;
    printf("LIBC-HOST checks=%d failures=%d\n", checks, failures);
    return failures;
}
