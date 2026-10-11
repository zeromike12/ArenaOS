/*
  test_startup.c: Host Unit Tests for Canonical ARST v2 Startup Gate
  Links and validates the ACTUAL arenaos_entry.c implementation directly!
*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>
#include "../SDL-release-3.2.0/src/platform/arenaos/arenaos_entry.h"

static void put16(uint8_t *p, size_t at, uint16_t v) {
    p[at] = (uint8_t)v;
    p[at + 1] = (uint8_t)(v >> 8);
}

static void put32(uint8_t *p, size_t at, uint32_t v) {
    p[at] = (uint8_t)v;
    p[at + 1] = (uint8_t)(v >> 8);
    p[at + 2] = (uint8_t)(v >> 16);
    p[at + 3] = (uint8_t)(v >> 24);
}

static void put64(uint8_t *p, size_t at, uint64_t v) {
    put32(p, at, (uint32_t)v);
    put32(p, at + 4, (uint32_t)(v >> 32));
}

static void build_canonical_page(uint8_t page[STARTUP_BLOCK_BYTES]) {
    memset(page, 0, STARTUP_BLOCK_BYTES);

    /* Magic & Version */
    page[0] = 'A'; page[1] = 'R'; page[2] = 'S'; page[3] = 'T';
    put16(page, 4, STARTUP_VERSION);
    put16(page, 6, STARTUP_HEADER_BYTES);

    uint32_t flags = FLAG_MULTI_INSTANCE;
    put32(page, 12, flags);

    /* App ID: "org.arenaos.test\0" */
    const char *id = "org.arenaos.test";
    memcpy(page + 16, id, strlen(id));

    put16(page, 48, 2);               /* instance_slot */
    put64(page, 52, 9999ULL);         /* instance_generation */

    uint16_t argc = 2;
    uint16_t envc = 1;
    uint16_t capc = 3;

    put16(page, 60, argc);
    put16(page, 62, envc);
    put16(page, 64, capc);

    uint32_t caps_off = 128;
    uint32_t args_off = caps_off + capc * STARTUP_CAPABILITY_DESCRIPTOR_BYTES;
    uint32_t env_off = args_off + argc * STARTUP_STRING_DESCRIPTOR_BYTES;
    uint32_t strings_off = env_off + envc * STARTUP_STRING_DESCRIPTOR_BYTES;

    put32(page, 68, args_off);
    put32(page, 72, env_off);
    put32(page, 76, caps_off);
    put32(page, 80, strings_off);

    put16(page, 88, STARTUP_NONE);    /* cwd */
    put16(page, 90, STARTUP_NONE);    /* stdin */
    put16(page, 92, STARTUP_NONE);    /* stdout */
    put16(page, 94, STARTUP_NONE);    /* stderr */

    put32(page, 96, STARTUP_BLOCK_BYTES);
    put64(page, 104, 0x200be8ULL);    /* entry */
    put64(page, 112, 0x200000ULL);    /* load_base */
    put64(page, 120, 12345678ULL);    /* clock_us */

    /* Capability descriptors: Slot 1 (Endpoint), Slot 2 (Surface), Slot 3 (Clock) */
    /* Slot 1: BadgedEndpoint WRITE */
    size_t d1 = caps_off;
    put16(page, d1, 1);
    page[d1 + 2] = CAP_ROLE_OTHER;
    page[d1 + 3] = CAP_KIND_BADGED_ENDPOINT;
    put32(page, d1 + 4, RIGHT_WRITE);

    /* Slot 2: SharedRegion READ | WRITE */
    size_t d2 = caps_off + 16;
    put16(page, d2, 2);
    page[d2 + 2] = CAP_ROLE_OTHER;
    page[d2 + 3] = CAP_KIND_SHARED_REGION;
    put32(page, d2 + 4, RIGHT_READ | RIGHT_WRITE);

    /* Slot 3: Notification READ | WRITE */
    size_t d3 = caps_off + 32;
    put16(page, d3, 3);
    page[d3 + 2] = CAP_ROLE_OTHER;
    page[d3 + 3] = CAP_KIND_NOTIFICATION;
    put32(page, d3 + 4, RIGHT_READ | RIGHT_WRITE);

    /* Strings payload:
       arg0: "bin/test_app" (len 12)
       arg1: "--debug"      (len 7)
       env0: "THEME=dark"   (len 10)
       Total strings_len = 12 + 7 + 10 = 29
    */
    const char *s_arg0 = "bin/test_app";
    const char *s_arg1 = "--debug";
    const char *s_env0 = "THEME=dark";

    uint32_t len0 = (uint32_t)strlen(s_arg0);
    uint32_t len1 = (uint32_t)strlen(s_arg1);
    uint32_t len2 = (uint32_t)strlen(s_env0);

    /* arg0 descriptor: offset 0, len 12 */
    put32(page, args_off, 0);
    put32(page, args_off + 4, len0);

    /* arg1 descriptor: offset 12, len 7 */
    put32(page, args_off + 8, len0);
    put32(page, args_off + 12, len1);

    /* env0 descriptor: offset 19, len 10 */
    put32(page, env_off, len0 + len1);
    put32(page, env_off + 4, len2);

    /* Write string bytes */
    memcpy(page + strings_off, s_arg0, len0);
    memcpy(page + strings_off + len0, s_arg1, len1);
    memcpy(page + strings_off + len0 + len1, s_env0, len2);

    uint32_t strings_len = len0 + len1 + len2;
    put32(page, 84, strings_len);

    uint32_t total = strings_off + strings_len;
    put32(page, 8, total);
}

int main(void) {
    printf("=== Testing Canonical ARST v2 Startup Gate Validation ===\n");

    uint8_t page[STARTUP_BLOCK_BYTES];
    arenaos_startup_record_t rec;
    char string_store[4096];
    char *argv[STARTUP_ARGUMENT_MAX + 1];
    char *envp[STARTUP_ENVIRONMENT_MAX + 1];

    /* Test 1: Canonical valid ARST v2 record */
    printf("[test_startup] Test 1: Canonical valid ARST v2 record...\n");
    build_canonical_page(page);
    int rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc == 0);
    assert(rec.argc == 2);
    assert(rec.envc == 1);
    assert(rec.cap_count == 3);
    assert(strcmp(argv[0], "bin/test_app") == 0);
    assert(strcmp(argv[1], "--debug") == 0);
    assert(argv[2] == NULL);
    assert(strcmp(envp[0], "THEME=dark") == 0);
    assert(envp[1] == NULL);
    printf("  [PASS] Canonical ARST v2 page verified successfully.\n");

    /* Test 2: Bad magic rejection */
    printf("[test_startup] Test 2: Bad magic rejection...\n");
    build_canonical_page(page);
    page[0] = 'X';
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Bad magic safely rejected.\n");

    /* Test 3: Bad version rejection */
    printf("[test_startup] Test 3: Bad version rejection...\n");
    build_canonical_page(page);
    put16(page, 4, 3); /* Version 3 unsupported */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Unsupported version safely rejected.\n");

    /* Test 4: Unknown flags rejection */
    printf("[test_startup] Test 4: Unknown flags rejection...\n");
    build_canonical_page(page);
    put32(page, 12, 0x80000000U); /* undefined flag */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Undefined flag bits safely rejected.\n");

    /* Test 5: Malformed app ID rejection */
    printf("[test_startup] Test 5: Malformed app ID rejection...\n");
    build_canonical_page(page);
    page[16] = '.'; /* starts with dot */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Malformed app ID safely rejected.\n");

    /* Test 6: Non-zero header padding rejection */
    printf("[test_startup] Test 6: Non-zero header padding rejection...\n");
    build_canonical_page(page);
    page[50] = 0xAA;
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Non-zero header padding safely rejected.\n");

    /* Test 7: Non-canonical capability slot rejection */
    printf("[test_startup] Test 7: Non-canonical capability slot rejection...\n");
    build_canonical_page(page);
    put16(page, 128, 2); /* Slot 1 specified as 2 */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Non-consecutive capability slot safely rejected.\n");

    /* Test 8: Tampered section offset rejection */
    printf("[test_startup] Test 8: Tampered offset rejection...\n");
    build_canonical_page(page);
    put32(page, 68, 128); /* args offset overlapping caps */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Tampered section offset safely rejected.\n");

    /* Test 9: Argument offset outside string table */
    printf("[test_startup] Test 9: Argument offset outside string table...\n");
    build_canonical_page(page);
    uint32_t args_off = 128 + 3 * 16;
    put32(page, args_off + 8, 500); /* arg1 offset past strings_len */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Out-of-bounds argument offset safely rejected.\n");

    /* Test 10: Argument length extending past string table */
    printf("[test_startup] Test 10: Argument length extending past string table...\n");
    build_canonical_page(page);
    put32(page, args_off + 12, 1000); /* arg1 length extends past strings_len */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Extended argument length safely rejected.\n");

    /* Test 11: Integer arithmetic overflow in offset + length */
    printf("[test_startup] Test 11: Integer arithmetic overflow in string offset/len...\n");
    build_canonical_page(page);
    put32(page, args_off, 0xFFFFFFF0U);
    put32(page, args_off + 4, 0x20U); /* overflow wraps around */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Integer arithmetic overflow safely rejected.\n");

    /* Test 12: Non-canonical out-of-order / overlapping string descriptors */
    printf("[test_startup] Test 12: Non-canonical out-of-order descriptors...\n");
    build_canonical_page(page);
    put32(page, args_off + 8, 0); /* arg1 points back to offset 0 instead of 12 */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Non-canonical overlapping descriptor safely rejected.\n");

    /* Test 13: Embedded NUL byte in argument string */
    printf("[test_startup] Test 13: Embedded NUL byte in argument...\n");
    build_canonical_page(page);
    uint32_t strings_off = 128 + 3 * 16 + 2 * 8 + 1 * 8;
    page[strings_off + 3] = '\0'; /* embedded NUL in "bin/test_app" */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Embedded NUL byte safely rejected.\n");

    /* Test 14: Empty argv[0] */
    printf("[test_startup] Test 14: Empty argv[0] rejection...\n");
    build_canonical_page(page);
    put32(page, args_off + 4, 0); /* len 0 for arg0 */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Empty argv[0] safely rejected.\n");

    /* Test 15: Empty environment string */
    printf("[test_startup] Test 15: Empty environment string rejection...\n");
    build_canonical_page(page);
    uint32_t env_off = args_off + 2 * 8;
    put32(page, env_off + 4, 0); /* len 0 for env0 */
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Empty environment string safely rejected.\n");

    /* Test 16: Oversized cumulative storage rejection */
    printf("[test_startup] Test 16: Destination buffer capacity check...\n");
    build_canonical_page(page);
    char small_store[10]; /* smaller than required 29 + 2 + 1 + 2 = 34 bytes */
    rc = arenaos_validate_and_parse_arst(page, &rec, small_store, sizeof(small_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Oversized cumulative storage safely refused.\n");

    /* Test 17: Incomplete string table coverage (expected_offset != strings_len) */
    printf("[test_startup] Test 17: Incomplete string table coverage...\n");
    build_canonical_page(page);
    put32(page, 84, 50); /* strings_len claiming 50, but descriptors only sum to 29 */
    put32(page, 8, strings_off + 50);
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Incomplete string table coverage safely rejected.\n");

    /* Test 18: Duplicate unique capability roles */
    printf("[test_startup] Test 18: Duplicate unique capability role rejection...\n");
    build_canonical_page(page);
    page[128 + 2] = CAP_ROLE_CURRENT_DIR; /* slot 1 role CurrentDirectory */
    page[128 + 3] = CAP_KIND_BADGED_ENDPOINT;
    put32(page, 128 + 4, RIGHT_WRITE);
    page[128 + 16 + 2] = CAP_ROLE_CURRENT_DIR; /* slot 2 ALSO role CurrentDirectory */
    page[128 + 16 + 3] = CAP_KIND_BADGED_ENDPOINT;
    put32(page, 128 + 16 + 4, RIGHT_WRITE);
    rc = arenaos_validate_and_parse_arst(page, &rec, string_store, sizeof(string_store), argv, envp);
    assert(rc != 0);
    printf("  [PASS] Duplicate unique role safely rejected.\n");

    printf("\nALL 18 CANONICAL STARTUP VALIDATION TESTS PASSED!\n");
    return 0;
}
