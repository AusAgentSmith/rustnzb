//! GF(2^16) Galois field arithmetic.
//!
//! PAR2 mandates the irreducible polynomial `0x1100B`:
//!   x^16 + x^12 + x^3 + x + 1
//!
//! All arithmetic uses log/antilog (exp) tables for O(1) multiply/divide.
//! The generator element is 2 (as per the PAR2 spec).

use std::sync::OnceLock;

/// The irreducible polynomial for GF(2^16) as used by PAR2.
/// x^16 + x^12 + x^3 + x + 1 = 0x1100B
const POLYNOMIAL: u32 = 0x1100B;

/// Field size: 2^16 = 65536 elements.
const FIELD_SIZE: usize = 65536;

/// Order of the multiplicative group: 2^16 - 1 = 65535.
const FIELD_ORDER: usize = 65535;

/// Precomputed tables for GF(2^16) arithmetic.
struct GfTables {
    /// log_table[x] = discrete log base 2 of x (undefined for x=0).
    log_table: Vec<u16>,
    /// exp_table[i] = 2^i mod polynomial. Double-sized for modular index wrapping.
    exp_table: Vec<u16>,
}

static TABLES: OnceLock<GfTables> = OnceLock::new();

fn tables() -> &'static GfTables {
    TABLES.get_or_init(|| {
        let mut log_table = vec![0u16; FIELD_SIZE];
        let mut exp_table = vec![0u16; FIELD_ORDER * 2];

        // Build exp table: exp[i] = g^i where g = 2 (the generator)
        let mut val: u32 = 1;
        for i in 0..FIELD_ORDER {
            exp_table[i] = val as u16;
            // Multiply by generator (2) in GF(2^16)
            val <<= 1;
            if val & 0x10000 != 0 {
                val ^= POLYNOMIAL;
            }
        }

        // Extend exp table for easy modular reduction: exp[i + FIELD_ORDER] = exp[i]
        for i in 0..FIELD_ORDER {
            exp_table[i + FIELD_ORDER] = exp_table[i];
        }

        // Build log table: log[exp[i]] = i
        for i in 0..FIELD_ORDER {
            log_table[exp_table[i] as usize] = i as u16;
        }
        // log[0] is undefined but we set it to 0 for safety
        log_table[0] = 0;

        GfTables {
            log_table,
            exp_table,
        }
    })
}

/// Addition in GF(2^16) is XOR.
#[inline]
pub fn add(a: u16, b: u16) -> u16 {
    a ^ b
}

/// Subtraction in GF(2^16) is the same as addition (XOR).
#[inline]
pub fn sub(a: u16, b: u16) -> u16 {
    a ^ b
}

/// Multiply two elements in GF(2^16).
/// Returns 0 if either operand is 0.
#[inline]
pub fn mul(a: u16, b: u16) -> u16 {
    if a == 0 || b == 0 {
        return 0;
    }
    let t = tables();
    let log_sum = t.log_table[a as usize] as usize + t.log_table[b as usize] as usize;
    t.exp_table[log_sum]
}

/// Divide a by b in GF(2^16).
/// Panics if b is 0 (division by zero).
#[inline]
pub fn div(a: u16, b: u16) -> u16 {
    assert!(b != 0, "GF(2^16) division by zero");
    if a == 0 {
        return 0;
    }
    let t = tables();
    let log_a = t.log_table[a as usize] as usize;
    let log_b = t.log_table[b as usize] as usize;
    // Use extended table to avoid negative index: log_a + (FIELD_ORDER - log_b)
    let idx = log_a + FIELD_ORDER - log_b;
    t.exp_table[idx]
}

/// Multiplicative inverse: inv(a) = a^(FIELD_ORDER-1) = a^65534.
/// Panics if a is 0.
#[inline]
pub fn inv(a: u16) -> u16 {
    assert!(a != 0, "GF(2^16) inverse of zero");
    let t = tables();
    let log_a = t.log_table[a as usize] as usize;
    t.exp_table[FIELD_ORDER - log_a]
}

/// Raise a to the power exp in GF(2^16).
#[inline]
pub fn pow(a: u16, exp: u32) -> u16 {
    if a == 0 {
        return 0;
    }
    if exp == 0 {
        return 1;
    }
    let t = tables();
    let log_a = t.log_table[a as usize] as u64;
    let log_result = (log_a * exp as u64) % FIELD_ORDER as u64;
    t.exp_table[log_result as usize]
}

/// Return 2^exp in GF(2^16). The generator is 2.
#[inline]
pub fn exp2(exp: u32) -> u16 {
    let t = tables();
    t.exp_table[exp as usize % FIELD_ORDER]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_is_xor() {
        assert_eq!(add(0, 0), 0);
        assert_eq!(add(1, 1), 0);
        assert_eq!(add(0xFFFF, 0), 0xFFFF);
        assert_eq!(add(0xAAAA, 0x5555), 0xFFFF);
    }

    #[test]
    fn test_mul_identity() {
        // Multiply by 1 is identity
        for x in [0u16, 1, 2, 100, 1000, 65535] {
            assert_eq!(mul(x, 1), x, "mul({x}, 1) should be {x}");
            assert_eq!(mul(1, x), x, "mul(1, {x}) should be {x}");
        }
    }

    #[test]
    fn test_mul_zero() {
        for x in [0u16, 1, 2, 100, 65535] {
            assert_eq!(mul(x, 0), 0);
            assert_eq!(mul(0, x), 0);
        }
    }

    #[test]
    fn test_mul_commutative() {
        for a in [2u16, 3, 100, 1000, 32768, 65535] {
            for b in [2u16, 3, 100, 1000, 32768, 65535] {
                assert_eq!(mul(a, b), mul(b, a), "mul({a}, {b}) should be commutative");
            }
        }
    }

    #[test]
    fn test_inverse() {
        // a * inv(a) should always equal 1
        for a in [1u16, 2, 3, 100, 1000, 32768, 65535] {
            let a_inv = inv(a);
            assert_eq!(mul(a, a_inv), 1, "a={a}, inv(a)={a_inv}: a*inv(a) should be 1");
        }
    }

    #[test]
    fn test_div_inverse_of_mul() {
        for a in [1u16, 2, 42, 1000, 65535] {
            for b in [1u16, 2, 42, 1000, 65535] {
                let product = mul(a, b);
                assert_eq!(div(product, b), a, "div(mul({a},{b}), {b}) should be {a}");
            }
        }
    }

    #[test]
    fn test_pow_basics() {
        // a^0 = 1
        assert_eq!(pow(42, 0), 1);
        // a^1 = a
        assert_eq!(pow(42, 1), 42);
        // 0^n = 0 for n > 0
        assert_eq!(pow(0, 5), 0);
        // 1^n = 1
        assert_eq!(pow(1, 999), 1);
    }

    #[test]
    fn test_pow_consistent_with_mul() {
        // a^2 = a * a
        for a in [2u16, 3, 100, 65535] {
            assert_eq!(pow(a, 2), mul(a, a));
            assert_eq!(pow(a, 3), mul(mul(a, a), a));
        }
    }

    #[test]
    fn test_generator_order() {
        // The generator 2 should have order FIELD_ORDER = 65535
        // 2^65535 = 1
        assert_eq!(pow(2, FIELD_ORDER as u32), 1);
        // 2^32768 != 1 (not a subgroup generator)
        assert_ne!(pow(2, 32768), 1);
    }

    #[test]
    fn test_exp2() {
        assert_eq!(exp2(0), 1);
        assert_eq!(exp2(1), 2);
        assert_eq!(exp2(2), 4);
        // exp2 should wrap around
        assert_eq!(exp2(FIELD_ORDER as u32), 1);
    }

    #[test]
    fn test_field_closure() {
        // Multiply all non-zero elements by a fixed non-zero element
        // should produce a permutation of all non-zero elements
        let a = 42u16;
        let mut seen = vec![false; FIELD_SIZE];
        for x in 1..FIELD_SIZE as u16 {
            let product = mul(a, x);
            assert!(!seen[product as usize], "duplicate product for a={a}, x={x}");
            seen[product as usize] = true;
        }
        // 0 should not be produced (since a != 0 and x != 0)
        assert!(!seen[0]);
    }

    /// Verify against known par2cmdline values.
    /// These were computed using par2cmdline-turbo's galois.cpp.
    #[test]
    fn test_known_par2_values() {
        // Generator powers
        assert_eq!(pow(2, 0), 1);
        assert_eq!(pow(2, 1), 2);
        assert_eq!(pow(2, 8), 256);
        assert_eq!(pow(2, 16), 0x100B); // x^16 mod polynomial = x^12 + x^3 + x + 1 = 0x100B
    }
}
