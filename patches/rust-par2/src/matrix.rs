//! Matrix operations over GF(2^16) for Reed-Solomon decoding.
//!
//! PAR2 repair requires:
//! 1. Building a Vandermonde-like matrix from recovery block exponents
//! 2. Selecting rows corresponding to available (non-damaged) data + recovery blocks
//! 3. Inverting the submatrix via Gaussian elimination
//! 4. Multiplying the inverse by recovery data to recover original blocks

use crate::gf;

/// A matrix over GF(2^16), stored in row-major order.
#[derive(Debug, Clone)]
pub struct GfMatrix {
    pub rows: usize,
    pub cols: usize,
    /// Row-major data: element at (r, c) is data[r * cols + c].
    pub data: Vec<u16>,
}

impl GfMatrix {
    /// Create a zero matrix.
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![0u16; rows * cols],
        }
    }

    /// Create an identity matrix.
    pub fn identity(n: usize) -> Self {
        let mut m = Self::zeros(n, n);
        for i in 0..n {
            m.set(i, i, 1);
        }
        m
    }

    /// Get element at (row, col).
    #[inline]
    pub fn get(&self, row: usize, col: usize) -> u16 {
        self.data[row * self.cols + col]
    }

    /// Set element at (row, col).
    #[inline]
    pub fn set(&mut self, row: usize, col: usize, val: u16) {
        self.data[row * self.cols + col] = val;
    }

    /// Build the PAR2 encoding matrix.
    ///
    /// PAR2 recovery blocks use: `recovery[e] = Σ (input[i] * c[i]^e)`
    /// where `c[i]` are per-input-slice constants assigned by the PAR2 spec.
    ///
    /// The encoding matrix has `input_count` columns (one per data block).
    /// Row `i` for i < input_count is the identity (data blocks are passed through).
    /// Row `input_count + r` is the recovery row for exponent `recovery_exponents[r]`:
    ///   `[c[0]^exp, c[1]^exp, c[2]^exp, ..., c[k-1]^exp]`
    pub fn par2_encoding_matrix(input_count: usize, recovery_exponents: &[u32]) -> Self {
        let total_rows = input_count + recovery_exponents.len();
        let mut m = Self::zeros(total_rows, input_count);

        // Compute input slice constants per PAR2 spec
        let constants = par2_input_constants(input_count);

        // Identity rows for data blocks
        for i in 0..input_count {
            m.set(i, i, 1);
        }

        // Recovery rows: row[input_count + r][c] = constants[c] ^ exponent[r]
        for (r, &exp) in recovery_exponents.iter().enumerate() {
            for c in 0..input_count {
                let val = gf::pow(constants[c], exp);
                m.set(input_count + r, c, val);
            }
        }

        m
    }

    /// Select specific rows from this matrix, returning a new matrix.
    pub fn select_rows(&self, row_indices: &[usize]) -> Self {
        let mut result = Self::zeros(row_indices.len(), self.cols);
        for (new_row, &old_row) in row_indices.iter().enumerate() {
            let src_start = old_row * self.cols;
            let dst_start = new_row * self.cols;
            result.data[dst_start..dst_start + self.cols]
                .copy_from_slice(&self.data[src_start..src_start + self.cols]);
        }
        result
    }

    /// Invert this square matrix using Gaussian elimination over GF(2^16).
    /// Returns None if the matrix is singular.
    pub fn invert(&self) -> Option<Self> {
        assert_eq!(self.rows, self.cols, "Can only invert square matrices");
        let n = self.rows;

        // Augmented matrix [A | I]
        let mut aug = Self::zeros(n, 2 * n);
        for r in 0..n {
            for c in 0..n {
                aug.set(r, c, self.get(r, c));
            }
            aug.set(r, n + r, 1); // Identity on the right
        }

        // Forward elimination (row echelon form)
        for col in 0..n {
            // Find pivot row
            let mut pivot_row = None;
            for r in col..n {
                if aug.get(r, col) != 0 {
                    pivot_row = Some(r);
                    break;
                }
            }
            let pivot_row = pivot_row?; // Singular if no pivot found

            // Swap pivot row into position
            if pivot_row != col {
                for c in 0..2 * n {
                    let tmp = aug.get(col, c);
                    aug.set(col, c, aug.get(pivot_row, c));
                    aug.set(pivot_row, c, tmp);
                }
            }

            // Scale pivot row so pivot element = 1
            let pivot_val = aug.get(col, col);
            let pivot_inv = gf::inv(pivot_val);
            for c in 0..2 * n {
                aug.set(col, c, gf::mul(aug.get(col, c), pivot_inv));
            }

            // Eliminate column in all other rows
            for r in 0..n {
                if r == col {
                    continue;
                }
                let factor = aug.get(r, col);
                if factor == 0 {
                    continue;
                }
                for c in 0..2 * n {
                    let val = gf::add(aug.get(r, c), gf::mul(factor, aug.get(col, c)));
                    aug.set(r, c, val);
                }
            }
        }

        // Extract the inverse (right half of augmented matrix)
        let mut result = Self::zeros(n, n);
        for r in 0..n {
            for c in 0..n {
                result.set(r, c, aug.get(r, n + c));
            }
        }

        Some(result)
    }
}

/// Compute the PAR2 input slice constants.
///
/// Per the PAR2 spec, each input slice is assigned a constant `c[i] = 2^n[i]`
/// where `n[i]` is the i-th valid exponent. Valid exponents satisfy:
///   `n % 3 != 0 && n % 5 != 0 && n % 17 != 0 && n % 257 != 0`
///
/// This ensures all constants have order 65535 (the full multiplicative group),
/// which guarantees the Vandermonde matrix is non-singular.
pub fn par2_input_constants(count: usize) -> Vec<u16> {
    let mut constants = Vec::with_capacity(count);
    let mut n: u32 = 0;
    while constants.len() < count {
        n += 1;
        if n % 3 != 0 && n % 5 != 0 && n % 17 != 0 && n % 257 != 0 {
            constants.push(gf::exp2(n));
        }
    }
    constants
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_inverse() {
        let id = GfMatrix::identity(4);
        let inv = id.invert().unwrap();
        for r in 0..4 {
            for c in 0..4 {
                let expected = if r == c { 1 } else { 0 };
                assert_eq!(inv.get(r, c), expected);
            }
        }
    }

    #[test]
    fn test_inverse_roundtrip() {
        // Create a known non-singular matrix and verify A * A^-1 = I
        let mut m = GfMatrix::zeros(3, 3);
        m.set(0, 0, 1); m.set(0, 1, 2); m.set(0, 2, 3);
        m.set(1, 0, 4); m.set(1, 1, 5); m.set(1, 2, 6);
        m.set(2, 0, 7); m.set(2, 1, 8); m.set(2, 2, 10);

        let inv = m.invert().unwrap();

        // Verify M * M^-1 = I
        for r in 0..3 {
            for c in 0..3 {
                let mut sum = 0u16;
                for k in 0..3 {
                    sum = gf::add(sum, gf::mul(m.get(r, k), inv.get(k, c)));
                }
                let expected = if r == c { 1 } else { 0 };
                assert_eq!(sum, expected, "M*M^-1 [{r},{c}] should be {expected}");
            }
        }
    }

    #[test]
    fn test_vandermonde_invertible() {
        // A Vandermonde matrix with distinct generators is always invertible
        let exponents = vec![0, 1, 2];
        let m = GfMatrix::par2_encoding_matrix(3, &exponents);
        // The recovery submatrix (rows 3..6) should be invertible
        let recovery = m.select_rows(&[3, 4, 5]);
        assert!(recovery.invert().is_some(), "Vandermonde submatrix should be invertible");
    }

    #[test]
    fn test_select_rows() {
        let mut m = GfMatrix::zeros(4, 3);
        for r in 0..4 {
            for c in 0..3 {
                m.set(r, c, (r * 10 + c) as u16);
            }
        }
        let sub = m.select_rows(&[1, 3]);
        assert_eq!(sub.rows, 2);
        assert_eq!(sub.cols, 3);
        assert_eq!(sub.get(0, 0), 10);
        assert_eq!(sub.get(1, 2), 32);
    }

    #[test]
    fn test_singular_matrix() {
        // All-zero matrix is singular
        let m = GfMatrix::zeros(3, 3);
        assert!(m.invert().is_none());

        // Matrix with duplicate rows is singular
        let mut m = GfMatrix::zeros(2, 2);
        m.set(0, 0, 1); m.set(0, 1, 2);
        m.set(1, 0, 1); m.set(1, 1, 2);
        assert!(m.invert().is_none());
    }
}
