/*
 * C2.5 App B: a genuine upstream open-source library on the ArenaOS C runtime.
 *
 * The library is xxHash v0.8.4 (BSD 2-Clause), vendored UNMODIFIED under
 * third_party/xxhash-0.8.4 with hashes in SHA256SUMS. This file is only the
 * driver: it calls the library's public API and checks results.
 *
 * The same driver is built against glibc on the host (host/test_xxhash.sh path in
 * run.py). The corpus digest it prints must be identical in both builds.
 *
 * Requirements exercised: malloc/free (library state objects), memcpy/memset,
 * stdio output. No files, graphics, floating point, or dynamic loading.
 * Exit status: 63 when all checks pass, 64 otherwise, 58 if stdio cannot attach
 * (guest only; the host build never returns 58).
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "xxhash.h"
#if __STDC_HOSTED__ == 0
#include "arena/rt.h" /* guest only: stdio attach through the granted stream set */
#endif

#define APP_NAME "[xxhash-app]"
#define EXIT_PASS 63
#define EXIT_FAIL 64
#define EXIT_NO_STREAMS 58

static int checks;
static int failed;

#define EXPECT(cond, label)                                                   \
    do {                                                                       \
        checks++;                                                              \
        if (!(cond)) {                                                         \
            failed++;                                                          \
            printf(APP_NAME " check failed line %d: %s\n", __LINE__, (label)); \
        }                                                                      \
    } while (0)

static uint64_t rng_state = 0x9E3779B97F4A7C15ull;
static uint64_t rng(void) {
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 7;
    rng_state ^= rng_state << 17;
    return rng_state;
}

/* Known vectors from the upstream test suite (values for the empty and "abc" inputs). */
static void known_vectors(void) {
    EXPECT(XXH32("", 0, 0) == 0x02CC5D05u, "XXH32(\"\") known vector");
    EXPECT(XXH32("abc", 3, 0) == 0x32D153FFu, "XXH32(\"abc\") known vector");
    EXPECT(XXH64("", 0, 0) == 0xEF46DB3751D8E999ull, "XXH64(\"\") known vector");
    EXPECT(XXH64("abc", 3, 0) == 0x44BC2CF5AD770999ull, "XXH64(\"abc\") known vector");
    EXPECT(XXH3_64bits("", 0) == 0x2D06800538D394C2ull, "XXH3_64bits(\"\") known vector");
}

/* Streaming with random chunking must equal one-shot for every algorithm. */
static void streaming_equals_oneshot(void) {
    unsigned char data[1500];
    for (size_t i = 0; i < sizeof data; i++) {
        data[i] = (unsigned char)rng();
    }
    int bad = 0;
    for (size_t len = 0; len <= sizeof data; len += (len < 64 ? 1 : 37)) {
        XXH64_state_t *s64 = XXH64_createState();
        XXH32_state_t *s32 = XXH32_createState();
        XXH3_state_t *s3 = XXH3_createState();
        if (s64 == NULL || s32 == NULL || s3 == NULL) {
            bad++;
            XXH64_freeState(s64);
            XXH32_freeState(s32);
            XXH3_freeState(s3);
            continue;
        }
        XXH64_reset(s64, 0);
        XXH32_reset(s32, 0);
        XXH3_64bits_reset(s3);
        size_t off = 0;
        while (off < len) {
            size_t take = 1 + (size_t)(rng() % 97);
            if (take > len - off) {
                take = len - off;
            }
            XXH64_update(s64, data + off, take);
            XXH32_update(s32, data + off, take);
            XXH3_64bits_update(s3, data + off, take);
            off += take;
        }
        if (XXH64_digest(s64) != XXH64(data, len, 0) ||
            XXH32_digest(s32) != XXH32(data, len, 0) ||
            XXH3_64bits_digest(s3) != XXH3_64bits(data, len)) {
            bad++;
        }
        XXH64_freeState(s64);
        XXH32_freeState(s32);
        XXH3_freeState(s3);
    }
    EXPECT(bad == 0, "streaming digests equal one-shot digests");
}

/* Corpus digest: every algorithm over many lengths; a single value for the host/guest diff. */
static void corpus_digest(void) {
    unsigned char data[4096];
    for (size_t i = 0; i < sizeof data; i++) {
        data[i] = (unsigned char)(i * 131u + 7u);
    }
    uint64_t acc = 0;
    size_t inputs = 0;
    for (size_t len = 0; len <= sizeof data; len += (len < 256 ? 1 : 61)) {
        XXH128_hash_t h128 = XXH3_128bits(data, len);
        acc = XXH64(&acc, sizeof acc, 0) ^ XXH64(data, len, 0) ^ XXH3_64bits(data, len) ^ h128.low64 ^
              (h128.high64 * 0x9E3779B97F4A7C15ull) ^ (uint64_t)XXH32(data, len, (XXH32_hash_t)len);
        inputs++;
    }
    printf(APP_NAME " corpus inputs=%u digest=%016llx\n", (unsigned)inputs,
           (unsigned long long)acc);
    EXPECT(inputs > 0, "corpus is non-empty");
}

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;
#if __STDC_HOSTED__ == 0
    /* Without a stream grant nothing can be reported; the defined status is the signal. */
    if (arena_stdio_init() != 0) {
        return EXIT_NO_STREAMS;
    }
#endif
    printf(APP_NAME " xxHash 0.8.4 (upstream, unmodified) on the ArenaOS C runtime\n");
    known_vectors();
    streaming_equals_oneshot();
    corpus_digest();
    printf(APP_NAME " RESULT %s checks=%d failed=%d\n", failed == 0 ? "PASS" : "FAIL", checks, failed);
    return failed == 0 ? EXIT_PASS : EXIT_FAIL;
}
