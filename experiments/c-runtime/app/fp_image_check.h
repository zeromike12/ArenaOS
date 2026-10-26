/*
 * Phase 16: pure comparator for fp_probe_run FXSAVE64 images.
 *
 * Shared verbatim by the guest application (c_fp_app.c) and the host
 * negative-control test (build_fp_app.py compiles test_fp_image_check.c
 * with this header), so the exact logic that judges the guest is itself
 * tested against deliberately corrupted images on the host.
 *
 * Expected image after fp_probe_run loads its known state:
 *   bytes   0..1   x87 control word 0x037F (fninit)
 *   bytes   2..3   x87 status word: TOP field = 5 (three pushes), rest 0
 *   byte    4      abridged tag: bits 5,6,7 set (R5,R6,R7 non-empty) = 0xE0
 *   bytes  32..159 ST/MM register image, 16 bytes each. FXSAVE stores the
 *                  x87 stack in logical order ST(0)..ST(7) (verified by
 *                  executing the probe on host hardware): after pushes of
 *                  1.0, 2.5, 0.5 the slots hold ST(0)=0.5, ST(1)=2.5,
 *                  ST(2)=1.0 (80-bit, low 10 bytes). Slots ST(3)..ST(7)
 *                  are NOT judged: fninit empties registers through the
 *                  tag word without a data-zeroing guarantee, so emptiness
 *                  is judged by the tag/TOP rules above. (The abridged tag
 *                  byte is in physical order: TOP=5 -> R5,R6,R7 -> 0xE0.)
 *   bytes 160..287 XMM0..XMM7 equal to the caller's 8 x 16-byte patterns
 * XMM8..15, MXCSR and the reserved tail are intentionally NOT judged: the
 * probe does not claim control of them (the C caller may use them).
 */
#ifndef FP_IMAGE_CHECK_H
#define FP_IMAGE_CHECK_H

#include <stdint.h>
#include <string.h>

static inline uint16_t fp_rd16(const uint8_t *p, unsigned at) {
    uint16_t v;
    memcpy(&v, p + at, sizeof v);
    return v;
}

/* Returns 0 on an exact match, else the 1-based number of the rule that failed. */
static inline int fp_image_check(const uint8_t fx[512], const uint8_t xmm_vals[128]) {
    static const uint8_t x87_1_0[10] = {0, 0, 0, 0, 0, 0, 0, 0x80, 0xFF, 0x3F};
    static const uint8_t x87_2_5[10] = {0, 0, 0, 0, 0, 0, 0, 0xA0, 0x00, 0x40};
    static const uint8_t x87_0_5[10] = {0, 0, 0, 0, 0, 0, 0, 0x80, 0xFE, 0x3F};
    if (fp_rd16(fx, 0) != 0x037Fu) {
        return 1; /* control word */
    }
    if (fp_rd16(fx, 2) != (uint16_t)(5u << 11)) {
        return 2; /* status word TOP == 5, no flags raised */
    }
    if (fx[4] != 0xE0u) {
        return 3; /* abridged tag: exactly R5,R6,R7 live */
    }
    if (memcmp(fx + 32 + 0 * 16, x87_0_5, 10) != 0) {
        return 4; /* ST(0) = 0.5 (most recent push) */
    }
    if (memcmp(fx + 32 + 1 * 16, x87_2_5, 10) != 0) {
        return 5; /* ST(1) = 2.5 */
    }
    if (memcmp(fx + 32 + 2 * 16, x87_1_0, 10) != 0) {
        return 6; /* ST(2) = 1.0 */
    }
    if (memcmp(fx + 160, xmm_vals, 128) != 0) {
        return 7; /* XMM0..XMM7 patterns */
    }
    return 0;
}

#endif /* FP_IMAGE_CHECK_H */
