/*
 * ADR-0111 (proposed) ASTR v2 layout: standalone host model test.
 *
 * This file implements the *proposed* v2 ring (capacity 1024, mask
 * indexing) in isolation, plus the production v1 arithmetic (capacity 768,
 * modulo indexing), and runs both from counters seeded at 2^32-16. v2 must
 * preserve unread bytes and order across the wrap; v1 must be *detected*
 * corrupt on the identical scenario (negative control proving the model
 * exercises the real failure condition).
 *
 * Nothing here is production code or changes the wire format. Build:
 *   gcc -O1 -Wall -Wextra -Werror -fsanitize=address,undefined \
 *       test_stream_v2_model.c -o test && ./test
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int failures;
#define REQUIRE(cond)                                   \
    do {                                                \
        if (!(cond)) {                                  \
            failures++;                                 \
            printf("V2-MODEL FAIL line %d: %s\n", __LINE__, #cond); \
        }                                               \
    } while (0)

struct ring {
    uint32_t write, read;
    uint32_t capacity;
    int v2;
    uint8_t data[1024];
};

static uint32_t index_of(const struct ring *r, uint32_t counter) {
    return r->v2 ? (counter & (r->capacity - 1u)) : (counter % r->capacity);
}

static uint32_t occupied(const struct ring *r) { return r->write - r->read; }

static size_t ring_write(struct ring *r, const uint8_t *in, size_t n) {
    uint32_t occ = occupied(r);
    if (occ > r->capacity) return (size_t)-1;
    size_t avail = r->capacity - occ;
    if (n > avail) n = avail;
    uint32_t start = index_of(r, r->write);
    size_t first = n < (r->capacity - start) ? n : (r->capacity - start);
    memcpy(r->data + start, in, first);
    if (n > first) memcpy(r->data, in + first, n - first);
    r->write += (uint32_t)n;
    return n;
}

static size_t ring_read(struct ring *r, uint8_t *out, size_t n) {
    uint32_t occ = occupied(r);
    if (occ > r->capacity) return (size_t)-1;
    if (n > occ) n = occ;
    uint32_t start = index_of(r, r->read);
    size_t first = n < (r->capacity - start) ? n : (r->capacity - start);
    memcpy(out, r->data + start, first);
    if (n > first) memcpy(out + first, r->data, n - first);
    r->read += (uint32_t)n;
    return n;
}

/* Write 64 patterned bytes across the 2^32 wrap with 48 still unread from
 * before the seed point; return 1 if every byte is read back in order. */
static int scenario(int v2) {
    struct ring r;
    memset(&r, 0, sizeof r);
    r.capacity = v2 ? 1024 : 768;
    r.v2 = v2;
    uint8_t in[64], out[64];
    r.write = r.read = 0xFFFFFFD0u; /* 2^32 - 48 */
    for (unsigned i = 0; i < 48; i++) in[i] = (uint8_t)i;
    if (ring_write(&r, in, 48) != 48) return 0;      /* now at 2^32 exactly */
    for (unsigned i = 0; i < 16; i++) in[48 + i] = (uint8_t)(0xA0 + i);
    if (ring_write(&r, in + 48, 16) != 16) return 0; /* crosses the wrap */
    memset(out, 0, sizeof out);
    if (ring_read(&r, out, 64) != 64) return 0;
    for (unsigned i = 0; i < 48; i++) if (out[i] != (uint8_t)i) return 0;
    for (unsigned i = 0; i < 16; i++) if (out[48 + i] != (uint8_t)(0xA0 + i)) return 0;
    return occupied(&r) == 0;
}

int main(void) {
    REQUIRE(scenario(1) == 1);  /* v2: bytes and order preserved across wrap */
    REQUIRE(scenario(0) == 0);  /* v1 negative control: corruption detected */

    /* Full/empty exactly across the wrap, plus a 3 MiB pump (2+ wraps of
     * the index cycle; counters seeded high to also cross 2^32 again). */
    struct ring r;
    memset(&r, 0, sizeof r);
    r.capacity = 1024;
    r.v2 = 1;
    r.write = r.read = 0xFFFFFC00u; /* 2^32 - 1024 */
    uint8_t in[1024], out[1024];
    for (unsigned i = 0; i < 1024; i++) in[i] = (uint8_t)(i * 7u);
    REQUIRE(ring_write(&r, in, 1024) == 1024);
    REQUIRE(occupied(&r) == 1024);            /* full, counters now at 0 */
    REQUIRE(ring_write(&r, in, 1) == 0);      /* full refuses */
    REQUIRE(ring_read(&r, out, 1024) == 1024 && memcmp(in, out, 1024) == 0);
    REQUIRE(occupied(&r) == 0);
    uint32_t pumped = 0;
    for (int round = 0; round < 3072; round++) {
        for (unsigned i = 0; i < 1024; i++) in[i] = (uint8_t)(round + i);
        if (ring_write(&r, in, 1024) != 1024) break;
        if (ring_read(&r, out, 1024) != 1024 || memcmp(in, out, 1024) != 0) break;
        pumped += 1024;
    }
    REQUIRE(pumped == 3072u * 1024u);
    REQUIRE(r.write == r.read); /* both advanced 3 MiB past multiple wraps */

    if (failures == 0) {
        printf("V2-MODEL PASS (wrap preserve, v1 detected, full/empty, 3MiB pump)\n");
        return 0;
    }
    printf("V2-MODEL FAILURES=%d\n", failures);
    return 1;
}
