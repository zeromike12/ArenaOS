#include "../SDL-release-3.2.0/src/platform/arenaos/arenaos_ieee754.h"
#include <stdio.h>
#include <assert.h>
#include <math.h>

int main(void) {
    printf("=== Testing ArenaOS IEEE-754 Safe Bitwise Conversions ===\n");

    /* 1. Test Zero */
    assert(arenaos_int32_to_float_bits(0) == 0U);
    assert(arenaos_float_bits_to_int32(0U) == 0);
    assert(arenaos_float_bits_to_int32(0x80000000U) == 0); /* -0.0f */
    printf("  [PASS] Zero cases verified.\n");

    /* 2. Test Basic Integers */
    assert(arenaos_float_bits_to_int32(arenaos_int32_to_float_bits(1)) == 1);
    assert(arenaos_float_bits_to_int32(arenaos_int32_to_float_bits(-1)) == -1);
    assert(arenaos_float_bits_to_int32(arenaos_int32_to_float_bits(42)) == 42);
    assert(arenaos_float_bits_to_int32(arenaos_int32_to_float_bits(-42)) == -42);
    assert(arenaos_float_bits_to_int32(arenaos_int32_to_float_bits(1000000)) == 1000000);
    assert(arenaos_float_bits_to_int32(arenaos_int32_to_float_bits(-1000000)) == -1000000);
    printf("  [PASS] Basic integer round-trips verified.\n");

    /* 3. Test INT32_MAX and INT32_MIN Boundaries */
    uint32_t bits_max = arenaos_int32_to_float_bits(ARENAOS_INT32_MAX);
    uint32_t bits_min = arenaos_int32_to_float_bits(ARENAOS_INT32_MIN);

    /* (float)INT32_MAX rounds to 2147483648.0f (0x4F000000) */
    assert(bits_max == 0x4F000000U);
    /* (float)INT32_MIN is exact -2147483648.0f (0xCF000000) */
    assert(bits_min == 0xCF000000U);

    /* Conversion back to int32 saturates or matches */
    assert(arenaos_float_bits_to_int32(bits_max) == ARENAOS_INT32_MAX);
    assert(arenaos_float_bits_to_int32(bits_min) == ARENAOS_INT32_MIN);
    printf("  [PASS] INT32_MAX and INT32_MIN boundaries verified.\n");

    /* 4. Test Truncation Toward Zero for Fractions */
    /* 0.5f = 0x3F000000, -0.5f = 0xBF000000 */
    assert(arenaos_float_bits_to_int32(0x3F000000U) == 0);
    assert(arenaos_float_bits_to_int32(0xBF000000U) == 0);
    /* 1.75f: exp=0, frac=0x600000 -> 0x3FE00000 */
    assert(arenaos_float_bits_to_int32(0x3FE00000U) == 1);
    /* -1.75f: 0xBFE00000 */
    assert(arenaos_float_bits_to_int32(0xBFE00000U) == -1);
    printf("  [PASS] Fractional truncation toward zero verified.\n");

    /* 5. Test NaN and Infinities */
    uint32_t pos_inf = 0x7F800000U;
    uint32_t neg_inf = 0xFF800000U;
    uint32_t qnan    = 0x7FC00000U;
    uint32_t snan    = 0x7F800001U;

    assert(arenaos_float_bits_to_int32(pos_inf) == ARENAOS_INT32_MAX);
    assert(arenaos_float_bits_to_int32(neg_inf) == ARENAOS_INT32_MIN);
    assert(arenaos_float_bits_to_int32(qnan) == 0);
    assert(arenaos_float_bits_to_int32(snan) == 0);
    printf("  [PASS] NaN and Infinities safe handling verified.\n");

    /* 6. Test Subnormals and Extreme Exponents */
    uint32_t subnormal = 0x00000001U;
    assert(arenaos_float_bits_to_int32(subnormal) == 0);
    /* 1.0f * 2^40 (oversized exponent) */
    uint32_t huge_pos = ((127U + 40U) << 23);
    uint32_t huge_neg = 0x80000000U | ((127U + 40U) << 23);
    assert(arenaos_float_bits_to_int32(huge_pos) == ARENAOS_INT32_MAX);
    assert(arenaos_float_bits_to_int32(huge_neg) == ARENAOS_INT32_MIN);
    printf("  [PASS] Subnormals and extreme exponents safe handling verified.\n");

    printf("ALL IEEE-754 BITWISE TESTS PASSED!\n");
    return 0;
}
