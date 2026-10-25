/*
 * Host negative/positive controls for the Phase 16 FP image comparator.
 *
 * Compiled and run by build_fp_app.py on the host. Builds one synthetic
 * known-good FXSAVE image (per the layout contract in fp_image_check.h),
 * requires acceptance, then requires rejection for each single-aspect
 * corruption. This proves the guest judge can detect corrupted FP state;
 * it does not execute the probe or any syscall.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "../app/fp_image_check.h"

static int failures;

#define REQUIRE(cond, msg)                                              \
    do {                                                                \
        if (!(cond)) {                                                  \
            failures++;                                                 \
            printf("FP-IMAGE-CONTROL FAIL: %s\n", msg);                 \
        }                                                               \
    } while (0)

static void build_good(uint8_t fx[512], uint8_t xmm[128]) {
    static const uint8_t x87_1_0[10] = {0, 0, 0, 0, 0, 0, 0, 0x80, 0xFF, 0x3F};
    static const uint8_t x87_2_5[10] = {0, 0, 0, 0, 0, 0, 0, 0xA0, 0x00, 0x40};
    static const uint8_t x87_0_5[10] = {0, 0, 0, 0, 0, 0, 0, 0x80, 0xFE, 0x3F};
    memset(fx, 0, 512);
    for (unsigned i = 0; i < 128; i++) {
        xmm[i] = (uint8_t)(0x40 + i);
    }
    fx[0] = 0x7F;
    fx[1] = 0x03;                 /* control word 0x037F */
    fx[3] = (uint8_t)(5u << 3);   /* status TOP=5 in high byte (bits 11..13) */
    fx[4] = 0xE0;
    memcpy(fx + 32 + 2 * 16, x87_1_0, 10);
    memcpy(fx + 32 + 1 * 16, x87_2_5, 10);
    memcpy(fx + 32 + 0 * 16, x87_0_5, 10);
    memcpy(fx + 160, xmm, 128);
}

int main(void) {
    uint8_t fx[512], xmm[128], bad[512];
    build_good(fx, xmm);
    REQUIRE(fp_image_check(fx, xmm) == 0, "known-good image accepted");

    memcpy(bad, fx, 512);
    bad[160 + 3 * 16] ^= 0xFF;    /* the FP_PROBE_MUTATE corruption */
    REQUIRE(fp_image_check(bad, xmm) == 7, "corrupted XMM3 rejected (rule 7)");

    memcpy(bad, fx, 512);
    bad[32 + 0 * 16 + 9] ^= 0x01; /* ST(0) exponent byte */
    REQUIRE(fp_image_check(bad, xmm) == 4, "corrupted ST(0) rejected (rule 4)");

    memcpy(bad, fx, 512);
    bad[4] = 0xC0;                /* one x87 reg lost */
    REQUIRE(fp_image_check(bad, xmm) == 3, "tag loss rejected (rule 3)");

    memcpy(bad, fx, 512);
    bad[3] = 0;                   /* TOP reset to 0: state reinitialized */
    REQUIRE(fp_image_check(bad, xmm) == 2, "TOP reset rejected (rule 2)");

    memcpy(bad, fx, 512);
    memset(bad + 160, 0, 128);    /* XMM zeroed (fresh-thread image) */
    REQUIRE(fp_image_check(bad, xmm) == 7, "zeroed XMM rejected (rule 7)");

    memcpy(bad, fx, 512);
    memcpy(bad + 160, fx + 176, 112); /* XMM shifted by one: peer mix-up shape */
    REQUIRE(fp_image_check(bad, xmm) == 7, "shifted XMM rejected (rule 7)");

    if (failures == 0) {
        printf("FP-IMAGE-CONTROL PASS (7 cases)\n");
        return 0;
    }
    printf("FP-IMAGE-CONTROL FAILURES=%d\n", failures);
    return 1;
}
