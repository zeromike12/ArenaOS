#ifndef ARENAOS_IEEE754_H
#define ARENAOS_IEEE754_H

#include <stdint.h>
#include <stdbool.h>

#define ARENAOS_INT32_MAX 2147483647
#define ARENAOS_INT32_MIN (-2147483647 - 1)

/*
 * Pure integer bitwise conversion from int32_t to IEEE-754 single-precision float bits.
 * Guaranteed:
 * - 0 x87 / 0 SSE / 0 FPU instructions.
 * - Well-defined arithmetic on all inputs including INT32_MIN (-2147483648) and INT32_MAX.
 * - Standard IEEE-754 round-to-nearest, ties-to-even behavior.
 */
static inline uint32_t arenaos_int32_to_float_bits(int32_t val) {
    if (val == 0) return 0U;

    uint32_t sign;
    uint32_t uval;
    if (val < 0) {
        sign = 0x80000000U;
        uval = 0U - (uint32_t)val; /* Defined unsigned negation: INT32_MIN becomes 0x80000000U */
    } else {
        sign = 0U;
        uval = (uint32_t)val;
    }

    int msb = 31 - __builtin_clz(uval);
    uint32_t exp;
    uint32_t mantissa;

    if (msb <= 23) {
        int shift = 23 - msb;
        mantissa = (uval << shift) & 0x7FFFFFU;
        exp = (uint32_t)(msb + 127);
    } else {
        int shift = msb - 23;
        uint32_t base = uval >> shift;
        uint32_t round_bit = 1U << (shift - 1);
        uint32_t frac_bits = uval & ((1U << shift) - 1);

        /* Round to nearest, ties to even */
        if (frac_bits > round_bit || (frac_bits == round_bit && (base & 1U) != 0)) {
            base++;
        }

        /* Check if rounding overflowed mantissa bits (e.g. 0x00FFFFFF -> 0x01000000) */
        if (base & 0x01000000U) {
            base >>= 1;
            msb++;
        }

        exp = (uint32_t)(msb + 127);
        mantissa = base & 0x7FFFFFU;
    }

    return sign | (exp << 23) | mantissa;
}

/*
 * Pure integer bitwise conversion from IEEE-754 single-precision float bits to int32_t.
 * Guaranteed:
 * - 0 x87 / 0 SSE / 0 FPU instructions.
 * - Truncation toward zero matching standard C (int32_t)float cast.
 * - Safe handling of NaN (returns 0), Infinity (saturates to INT32_MIN/INT32_MAX),
 *   subnormals, oversized exponents, and INT32_MIN boundary without undefined shifts.
 */
static inline int32_t arenaos_float_bits_to_int32(uint32_t bits) {
    uint32_t sign = bits & 0x80000000U;
    uint32_t raw_exp = (bits >> 23) & 0xFFU;
    uint32_t frac = bits & 0x7FFFFFU;

    /* Handle NaN and Infinities */
    if (raw_exp == 255U) {
        if (frac != 0U) return 0; /* NaN */
        return sign ? ARENAOS_INT32_MIN : ARENAOS_INT32_MAX; /* Infinity */
    }

    /* Subnormal or zero: value magnitude is < 1.0 */
    if (raw_exp == 0U) {
        return 0;
    }

    int32_t exp = (int32_t)raw_exp - 127;
    /* Magnitude strictly less than 1.0 truncates to 0 */
    if (exp < 0) {
        return 0;
    }

    /* Exponents >= 32 overflow 32-bit signed int: saturate safely */
    if (exp >= 32) {
        return sign ? ARENAOS_INT32_MIN : ARENAOS_INT32_MAX;
    }

    /* Explicit leading 1 for normalized floats */
    uint32_t mantissa = frac | 0x800000U;

    if (exp < 23) {
        int shift = 23 - exp;
        uint32_t uval = mantissa >> shift;
        return sign ? -(int32_t)uval : (int32_t)uval;
    } else {
        int shift = exp - 23;
        uint32_t uval = mantissa << shift;
        if (sign) {
            if (uval >= 0x80000000U) return ARENAOS_INT32_MIN;
            return -(int32_t)uval;
        } else {
            if (uval >= (uint32_t)ARENAOS_INT32_MAX) return ARENAOS_INT32_MAX;
            return (int32_t)uval;
        }
    }
}

#endif /* ARENAOS_IEEE754_H */
