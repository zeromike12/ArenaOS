/*
  Host Unit Test for Canonical ARST v2 Startup Validation (Milestone G2.2)
*/

#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <assert.h>

#define STARTUP_BLOCK_BYTES 4096u
#define STARTUP_HEADER_BYTES 128u
#define STARTUP_CAPABILITY_MAX 7u
#define STARTUP_ARGUMENT_MAX 32u
#define STARTUP_ENVIRONMENT_MAX 32u
#define STARTUP_STRING_BYTES_MAX 3072u
#define STARTUP_VERSION 2u

#define CAP_KIND_NOTIFICATION 3u
#define CAP_KIND_SHARED_REGION 7u
#define CAP_KIND_BADGED_ENDPOINT 12u
#define CAP_KIND_SYNC_DOMAIN 16u

#define RIGHT_READ 1u
#define RIGHT_WRITE 2u
#define RIGHT_DESTROY 8u
#define RIGHTS_MASK 0x0Fu

#define FLAG_MULTI_INSTANCE (1u << 0)
#define FLAG_STANDARD_STREAMS (1u << 3)
#define FLAG_NATIVE_SYNC (1u << 4)
#define KNOWN_FLAGS (FLAG_MULTI_INSTANCE | (1u << 1) | (1u << 2) | FLAG_STANDARD_STREAMS | FLAG_NATIVE_SYNC)

static uint16_t rd16(const uint8_t *p, size_t at) {
    return (uint16_t)(p[at] | ((uint16_t)p[at + 1] << 8));
}

static uint32_t rd32(const uint8_t *p, size_t at) {
    return (uint32_t)p[at] | ((uint32_t)p[at + 1] << 8) | ((uint32_t)p[at + 2] << 16) | ((uint32_t)p[at + 3] << 24);
}

static uint64_t rd64(const uint8_t *p, size_t at) {
    return (uint64_t)rd32(p, at) | ((uint64_t)rd32(p, at + 4) << 32);
}

static int all_zero(const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (p[i] != 0) return 0;
    }
    return 1;
}

static int valid_app_id(const uint8_t *id) {
    size_t end = 0;
    while (end < 32 && id[end] != 0) {
        end++;
    }
    if (end < 1 || end > 31 || !all_zero(id + end, 32 - end)) {
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

static int parse_and_validate_record(const uint8_t *p) {
    if (p[0] != 'A' || p[1] != 'R' || p[2] != 'S' || p[3] != 'T') return -1;
    if (rd16(p, 4) != STARTUP_VERSION || rd16(p, 6) != STARTUP_HEADER_BYTES) return -1;

    if (p[50] || p[51] || p[66] || p[67] || p[100] || p[101] || p[102] || p[103]) return -1;

    uint32_t total = rd32(p, 8);
    uint32_t flags = rd32(p, 12);
    uint16_t argc = rd16(p, 60);
    uint16_t envc = rd16(p, 62);
    uint16_t capc = rd16(p, 64);
    uint32_t args_off = rd32(p, 68);
    uint32_t env_off = rd32(p, 72);
    uint32_t caps_off = rd32(p, 76);
    uint32_t strings_off = rd32(p, 80);
    uint32_t strings_len = rd32(p, 84);
    uint16_t instance_slot = rd16(p, 48);
    uint64_t generation = rd64(p, 52);

    if (total < STARTUP_HEADER_BYTES || total > STARTUP_BLOCK_BYTES ||
        argc == 0 || argc > STARTUP_ARGUMENT_MAX ||
        envc > STARTUP_ENVIRONMENT_MAX ||
        capc > STARTUP_CAPABILITY_MAX ||
        strings_len > STARTUP_STRING_BYTES_MAX ||
        instance_slot >= 32u || generation == 0) return -1;

    if (flags & ~KNOWN_FLAGS) return -1;
    if (!valid_app_id(p + 16)) return -1;

    uint32_t expect_args = STARTUP_HEADER_BYTES + (uint32_t)capc * 16u;
    uint32_t expect_env = expect_args + (uint32_t)argc * 8u;
    uint32_t expect_strings = expect_env + (uint32_t)envc * 8u;
    if (caps_off != STARTUP_HEADER_BYTES || args_off != expect_args ||
        env_off != expect_env || strings_off != expect_strings ||
        (uint64_t)strings_off + strings_len != total) return -1;

    if (!all_zero(p + total, STARTUP_BLOCK_BYTES - total)) return -1;

    for (unsigned i = 0; i < capc; i++) {
        size_t at = STARTUP_HEADER_BYTES + (size_t)i * 16u;
        uint16_t slot = rd16(p, at);
        uint8_t kind = p[at + 3];
        uint32_t rights = rd32(p, at + 4);

        if (slot != i + 1u || kind < 1 || kind > 16 || rights == 0 || (rights & ~RIGHTS_MASK)) return -1;
        if (!all_zero(p + at + 8, 8)) return -1;
    }

    return 0;
}

static void make_valid_page(uint8_t page[STARTUP_BLOCK_BYTES]) {
    memset(page, 0, STARTUP_BLOCK_BYTES);
    page[0] = 'A'; page[1] = 'R'; page[2] = 'S'; page[3] = 'T';
    page[4] = 2; page[5] = 0; // version = 2
    page[6] = 128; page[7] = 0; // header_bytes = 128

    // App ID
    const char app_id[] = "org.arenaos.sdl3app";
    memcpy(page + 16, app_id, strlen(app_id));

    page[48] = 0; page[49] = 0; // instance_slot = 0
    page[52] = 1; // generation = 1

    uint16_t argc = 1;
    uint16_t envc = 0;
    uint16_t capc = 3;

    page[60] = (uint8_t)(argc & 0xFF);
    page[62] = (uint8_t)(envc & 0xFF);
    page[64] = (uint8_t)(capc & 0xFF);

    uint32_t caps_off = 128;
    uint32_t args_off = caps_off + capc * 16; // 128 + 48 = 176
    uint32_t env_off = args_off + argc * 8;   // 176 + 8 = 184
    uint32_t strings_off = env_off + envc * 8;// 184 + 0 = 184

    const char arg0[] = "bin/sdl3_app";
    uint32_t strings_len = strlen(arg0);
    uint32_t total = strings_off + strings_len; // 184 + 12 = 196

    // Write total and offsets
    memcpy(page + 8, &total, 4);
    memcpy(page + 68, &args_off, 4);
    memcpy(page + 72, &env_off, 4);
    memcpy(page + 76, &caps_off, 4);
    memcpy(page + 80, &strings_off, 4);
    memcpy(page + 84, &strings_len, 4);

    // Caps
    // Cap 0 (slot 1): BadgedEndpoint (kind 12), WRITE (2)
    page[caps_off + 0] = 1; page[caps_off + 1] = 0;
    page[caps_off + 2] = 0; // role other
    page[caps_off + 3] = CAP_KIND_BADGED_ENDPOINT;
    page[caps_off + 4] = RIGHT_WRITE;

    // Cap 1 (slot 2): SharedRegion (kind 7), READ|WRITE (3)
    page[caps_off + 16 + 0] = 2; page[caps_off + 16 + 1] = 0;
    page[caps_off + 16 + 2] = 0; // role other
    page[caps_off + 16 + 3] = CAP_KIND_SHARED_REGION;
    page[caps_off + 16 + 4] = RIGHT_READ | RIGHT_WRITE;

    // Cap 2 (slot 3): Notification (kind 3), WRITE (2)
    page[caps_off + 32 + 0] = 3; page[caps_off + 32 + 1] = 0;
    page[caps_off + 32 + 2] = 0; // role other
    page[caps_off + 32 + 3] = CAP_KIND_NOTIFICATION;
    page[caps_off + 32 + 4] = RIGHT_WRITE;

    // Args: arg 0 offset 0, len = strlen(arg0)
    uint32_t a_off = 0;
    uint32_t a_len = strings_len;
    memcpy(page + args_off + 0, &a_off, 4);
    memcpy(page + args_off + 4, &a_len, 4);

    // Strings
    memcpy(page + strings_off, arg0, strings_len);
}

int main(void) {
    printf("=== Testing Canonical ARST v2 Startup Gate Validation ===\n");
    uint8_t page[STARTUP_BLOCK_BYTES];

    // Test 1: Valid canonical page
    printf("[test_startup] Test 1: Canonical valid ARST v2 record...\n");
    make_valid_page(page);
    assert(parse_and_validate_record(page) == 0);
    printf("  [PASS] Canonical ARST v2 page verified successfully.\n");

    // Test 2: Bad magic
    printf("[test_startup] Test 2: Bad magic rejection...\n");
    make_valid_page(page);
    page[0] = 'N'; page[1] = 'O'; page[2] = 'P'; page[3] = 'E';
    assert(parse_and_validate_record(page) == -1);
    printf("  [PASS] Bad magic safely rejected.\n");

    // Test 3: Bad version
    printf("[test_startup] Test 3: Bad version rejection...\n");
    make_valid_page(page);
    page[4] = 3; // version 3
    assert(parse_and_validate_record(page) == -1);
    printf("  [PASS] Unsupported version safely rejected.\n");

    // Test 4: Unknown flag
    printf("[test_startup] Test 4: Unknown flags rejection...\n");
    make_valid_page(page);
    page[12] = 0x80; // undefined high flag
    assert(parse_and_validate_record(page) == -1);
    printf("  [PASS] Undefined flag bits safely rejected.\n");

    // Test 5: Malformed app ID
    printf("[test_startup] Test 5: Malformed app ID rejection...\n");
    make_valid_page(page);
    page[16] = '@'; // uppercase or special char not allowed as first char
    assert(parse_and_validate_record(page) == -1);
    printf("  [PASS] Malformed app ID safely rejected.\n");

    // Test 6: Non-zero header padding
    printf("[test_startup] Test 6: Non-zero header padding rejection...\n");
    make_valid_page(page);
    page[50] = 1; // reserved padding byte
    assert(parse_and_validate_record(page) == -1);
    printf("  [PASS] Non-zero header padding safely rejected.\n");

    // Test 7: Non-canonical descriptor slot
    printf("[test_startup] Test 7: Non-canonical capability slot rejection...\n");
    make_valid_page(page);
    page[128 + 0] = 5; // slot 5 instead of slot 1
    assert(parse_and_validate_record(page) == -1);
    printf("  [PASS] Non-consecutive capability slot safely rejected.\n");

    // Test 8: Non-canonical offset
    printf("[test_startup] Test 8: Tampered offset rejection...\n");
    make_valid_page(page);
    uint32_t bad_args = 200;
    memcpy(page + 68, &bad_args, 4);
    assert(parse_and_validate_record(page) == -1);
    printf("  [PASS] Tampered section offset safely rejected.\n");

    printf("\nALL 8 STARTUP VALIDATION TESTS PASSED!\n");
    return 0;
}
