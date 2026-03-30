//! SIMD-accelerated GF(2^16) buffer operations.
//!
//! Dispatch hierarchy: AVX2 (256-bit) → SSSE3 (128-bit) → scalar.
//!
//! The PSHUFB technique: decompose 16-bit GF multiply into four 4-bit lookups.
//! VPSHUFB (AVX2) processes 32 bytes per instruction, PSHUFB (SSSE3) 16 bytes.
//!
//! Additionally provides `mul_add_multi` which accumulates multiple source
//! buffers × coefficients into dst in a single pass, reducing memory bandwidth
//! by loading each dst cache line once instead of once per source.

use crate::gf;

/// Precomputed PSHUFB tables for multiplying by a GF(2^16) constant.
pub struct GfMulTables {
    pub lo_lo: [u8; 16],  pub lo_hi: [u8; 16],
    pub hi_lo: [u8; 16],  pub hi_hi: [u8; 16],
    pub ulo_lo: [u8; 16], pub ulo_hi: [u8; 16],
    pub uhi_lo: [u8; 16], pub uhi_hi: [u8; 16],
}

impl GfMulTables {
    pub fn new(constant: u16) -> Self {
        let mut t = GfMulTables {
            lo_lo: [0; 16], lo_hi: [0; 16], hi_lo: [0; 16], hi_hi: [0; 16],
            ulo_lo: [0; 16], ulo_hi: [0; 16], uhi_lo: [0; 16], uhi_hi: [0; 16],
        };
        for i in 0..16u16 {
            let v = gf::mul(constant, i);
            t.lo_lo[i as usize] = v as u8;
            t.lo_hi[i as usize] = (v >> 8) as u8;
            let v = gf::mul(constant, i << 4);
            t.hi_lo[i as usize] = v as u8;
            t.hi_hi[i as usize] = (v >> 8) as u8;
            let v = gf::mul(constant, i << 8);
            t.ulo_lo[i as usize] = v as u8;
            t.ulo_hi[i as usize] = (v >> 8) as u8;
            let v = gf::mul(constant, i << 12);
            t.uhi_lo[i as usize] = v as u8;
            t.uhi_hi[i as usize] = (v >> 8) as u8;
        }
        t
    }
}

// =========================================================================
// Single-source: dst ^= constant * src
// =========================================================================

/// dst[i] ^= constant * src[i] for each u16 position.
pub fn mul_add_buffer(dst: &mut [u8], src: &[u8], constant: u16) {
    assert_eq!(dst.len(), src.len());
    if constant == 0 { return; }
    if constant == 1 { xor_buffers(dst, src); return; }

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { mul_add_buffer_avx2(dst, src, constant) };
            return;
        }
        if is_x86_feature_detected!("ssse3") {
            unsafe { mul_add_buffer_ssse3(dst, src, constant) };
            return;
        }
    }
    mul_add_buffer_scalar(dst, src, constant);
}

// =========================================================================
// Multi-source batched: dst ^= Σ coeffs[i] * srcs[i]
// =========================================================================

/// Accumulate multiple source buffers into dst.
///
/// `dst ^= coeffs[0]*srcs[0] + coeffs[1]*srcs[1] + ...`
///
/// Uses batched processing: groups sources into batches of 2, loading each
/// dst cache line once per batch instead of once per source. With AVX2
/// this halves memory bandwidth for dst.
pub fn mul_add_multi(dst: &mut [u8], srcs: &[&[u8]], coeffs: &[u16]) {
    assert_eq!(srcs.len(), coeffs.len());

    // Filter out zero coefficients
    let active: Vec<(usize, u16)> = coeffs.iter().copied().enumerate()
        .filter(|(_, c)| *c != 0)
        .collect();

    if active.is_empty() { return; }

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            // Process pairs of sources: load dst once, accumulate 2 sources, store
            let mut i = 0;
            while i + 1 < active.len() {
                let (idx1, c1) = active[i];
                let (idx2, c2) = active[i + 1];
                unsafe { mul_add_pair_avx2(dst, srcs[idx1], c1, srcs[idx2], c2) };
                i += 2;
            }
            // Odd remainder
            if i < active.len() {
                let (idx, c) = active[i];
                unsafe { mul_add_buffer_avx2(dst, srcs[idx], c) };
            }
            return;
        }
    }

    for &(idx, coeff) in &active {
        mul_add_buffer(dst, srcs[idx], coeff);
    }
}

/// XOR src into dst.
pub fn xor_buffers(dst: &mut [u8], src: &[u8]) {
    assert_eq!(dst.len(), src.len());
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { xor_buffers_avx2(dst, src) };
            return;
        }
    }
    for (d, s) in dst.iter_mut().zip(src.iter()) { *d ^= s; }
}

// =========================================================================
// Scalar fallback
// =========================================================================

fn mul_add_buffer_scalar(dst: &mut [u8], src: &[u8], constant: u16) {
    let len = dst.len() / 2;
    for i in 0..len {
        let off = i * 2;
        let s = u16::from_le_bytes([src[off], src[off + 1]]);
        let d = u16::from_le_bytes([dst[off], dst[off + 1]]);
        let result = d ^ gf::mul(constant, s);
        dst[off] = result as u8;
        dst[off + 1] = (result >> 8) as u8;
    }
}

// =========================================================================
// AVX2 implementation (256-bit = 32 bytes = 16 u16 values per instruction)
// =========================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn mul_add_buffer_avx2(dst: &mut [u8], src: &[u8], constant: u16) {
    use std::arch::x86_64::*;
    let tables = GfMulTables::new(constant);
    gf_mul_add_avx2_inner(dst, src, &tables);
}

/// Core AVX2 GF multiply-accumulate for a single src buffer.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn gf_mul_add_avx2_inner(dst: &mut [u8], src: &[u8], tables: &GfMulTables) {
    use std::arch::x86_64::*;

    // Broadcast 16-byte tables to 256-bit (duplicate in both 128-bit lanes)
    let nibble_mask = _mm256_set1_epi8(0x0F);
    let tbl_lo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.lo_lo.as_ptr() as *const _));
    let tbl_lo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.lo_hi.as_ptr() as *const _));
    let tbl_hi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.hi_lo.as_ptr() as *const _));
    let tbl_hi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.hi_hi.as_ptr() as *const _));
    let tbl_ulo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.ulo_lo.as_ptr() as *const _));
    let tbl_ulo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.ulo_hi.as_ptr() as *const _));
    let tbl_uhi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.uhi_lo.as_ptr() as *const _));
    let tbl_uhi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.uhi_hi.as_ptr() as *const _));

    // Deinterleave masks: extract even (u16-low) and odd (u16-high) bytes
    // VPSHUFB operates per 128-bit lane, so the mask is the same in both lanes
    let deint_lo = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        0,2,4,6,8,10,12,14, -1,-1,-1,-1,-1,-1,-1,-1));
    let deint_hi = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        1,3,5,7,9,11,13,15, -1,-1,-1,-1,-1,-1,-1,-1));

    let len = dst.len();
    let chunks = len / 32;

    for chunk in 0..chunks {
        let off = chunk * 32;
        let src_data = _mm256_loadu_si256(src[off..].as_ptr() as *const __m256i);

        // Deinterleave: separate u16-low and u16-high bytes within each 128-bit lane
        let src_lo_bytes = _mm256_shuffle_epi8(src_data, deint_lo);
        let src_hi_bytes = _mm256_shuffle_epi8(src_data, deint_hi);

        // Split into nibbles
        let lo_nib = _mm256_and_si256(src_lo_bytes, nibble_mask);
        let hi_nib = _mm256_and_si256(_mm256_srli_epi16(src_lo_bytes, 4), nibble_mask);
        let ulo_nib = _mm256_and_si256(src_hi_bytes, nibble_mask);
        let uhi_nib = _mm256_and_si256(_mm256_srli_epi16(src_hi_bytes, 4), nibble_mask);

        // VPSHUFB lookups — low byte of result
        let r_lo = _mm256_xor_si256(
            _mm256_xor_si256(_mm256_shuffle_epi8(tbl_lo_lo, lo_nib), _mm256_shuffle_epi8(tbl_hi_lo, hi_nib)),
            _mm256_xor_si256(_mm256_shuffle_epi8(tbl_ulo_lo, ulo_nib), _mm256_shuffle_epi8(tbl_uhi_lo, uhi_nib)),
        );
        // High byte of result
        let r_hi = _mm256_xor_si256(
            _mm256_xor_si256(_mm256_shuffle_epi8(tbl_lo_hi, lo_nib), _mm256_shuffle_epi8(tbl_hi_hi, hi_nib)),
            _mm256_xor_si256(_mm256_shuffle_epi8(tbl_ulo_hi, ulo_nib), _mm256_shuffle_epi8(tbl_uhi_hi, uhi_nib)),
        );

        // Interleave result bytes back to u16 LE (within each 128-bit lane)
        let result = _mm256_unpacklo_epi8(r_lo, r_hi);

        // XOR into destination
        let dst_val = _mm256_loadu_si256(dst[off..].as_ptr() as *const __m256i);
        _mm256_storeu_si256(dst[off..].as_mut_ptr() as *mut __m256i, _mm256_xor_si256(dst_val, result));
    }

    // Scalar remainder
    let rem = chunks * 32;
    if rem < len {
        mul_add_buffer_scalar(&mut dst[rem..], &src[rem..], gf::mul(
            // Recompute constant from tables — just pass it through
            // Actually we need the original constant, extract from table:
            // tables.lo_lo[1] | (tables.lo_hi[1] << 8) = constant * 1 = constant
            tables.lo_lo[1] as u16 | ((tables.lo_hi[1] as u16) << 8),
            1,
        ));
        // Simpler: just redo scalar with the real constant
    }
}

/// Process 2 sources per dst load/store: dst ^= c1*src1 + c2*src2
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn mul_add_pair_avx2(dst: &mut [u8], src1: &[u8], c1: u16, src2: &[u8], c2: u16) {
    use std::arch::x86_64::*;

    let t1 = GfMulTables::new(c1);
    let t2 = GfMulTables::new(c2);

    let nibble_mask = _mm256_set1_epi8(0x0F);
    let deint_lo = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        0,2,4,6,8,10,12,14, -1,-1,-1,-1,-1,-1,-1,-1));
    let deint_hi = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        1,3,5,7,9,11,13,15, -1,-1,-1,-1,-1,-1,-1,-1));

    // Load tables for source 1
    let t1_lo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.lo_lo.as_ptr() as *const _));
    let t1_lo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.lo_hi.as_ptr() as *const _));
    let t1_hi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.hi_lo.as_ptr() as *const _));
    let t1_hi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.hi_hi.as_ptr() as *const _));
    let t1_ulo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.ulo_lo.as_ptr() as *const _));
    let t1_ulo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.ulo_hi.as_ptr() as *const _));
    let t1_uhi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.uhi_lo.as_ptr() as *const _));
    let t1_uhi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t1.uhi_hi.as_ptr() as *const _));

    // Load tables for source 2
    let t2_lo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.lo_lo.as_ptr() as *const _));
    let t2_lo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.lo_hi.as_ptr() as *const _));
    let t2_hi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.hi_lo.as_ptr() as *const _));
    let t2_hi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.hi_hi.as_ptr() as *const _));
    let t2_ulo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.ulo_lo.as_ptr() as *const _));
    let t2_ulo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.ulo_hi.as_ptr() as *const _));
    let t2_uhi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.uhi_lo.as_ptr() as *const _));
    let t2_uhi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(t2.uhi_hi.as_ptr() as *const _));

    let len = dst.len();
    let chunks = len / 32;

    for chunk in 0..chunks {
        let off = chunk * 32;

        // Load dst once
        let mut acc = _mm256_loadu_si256(dst[off..].as_ptr() as *const __m256i);

        // Source 1
        let s1 = _mm256_loadu_si256(src1[off..].as_ptr() as *const __m256i);
        let s1_lo = _mm256_shuffle_epi8(s1, deint_lo);
        let s1_hi = _mm256_shuffle_epi8(s1, deint_hi);
        let n1 = _mm256_and_si256(s1_lo, nibble_mask);
        let n2 = _mm256_and_si256(_mm256_srli_epi16(s1_lo, 4), nibble_mask);
        let n3 = _mm256_and_si256(s1_hi, nibble_mask);
        let n4 = _mm256_and_si256(_mm256_srli_epi16(s1_hi, 4), nibble_mask);
        let r1_lo = _mm256_xor_si256(
            _mm256_xor_si256(_mm256_shuffle_epi8(t1_lo_lo, n1), _mm256_shuffle_epi8(t1_hi_lo, n2)),
            _mm256_xor_si256(_mm256_shuffle_epi8(t1_ulo_lo, n3), _mm256_shuffle_epi8(t1_uhi_lo, n4)),
        );
        let r1_hi = _mm256_xor_si256(
            _mm256_xor_si256(_mm256_shuffle_epi8(t1_lo_hi, n1), _mm256_shuffle_epi8(t1_hi_hi, n2)),
            _mm256_xor_si256(_mm256_shuffle_epi8(t1_ulo_hi, n3), _mm256_shuffle_epi8(t1_uhi_hi, n4)),
        );
        acc = _mm256_xor_si256(acc, _mm256_unpacklo_epi8(r1_lo, r1_hi));

        // Source 2
        let s2 = _mm256_loadu_si256(src2[off..].as_ptr() as *const __m256i);
        let s2_lo = _mm256_shuffle_epi8(s2, deint_lo);
        let s2_hi = _mm256_shuffle_epi8(s2, deint_hi);
        let n1 = _mm256_and_si256(s2_lo, nibble_mask);
        let n2 = _mm256_and_si256(_mm256_srli_epi16(s2_lo, 4), nibble_mask);
        let n3 = _mm256_and_si256(s2_hi, nibble_mask);
        let n4 = _mm256_and_si256(_mm256_srli_epi16(s2_hi, 4), nibble_mask);
        let r2_lo = _mm256_xor_si256(
            _mm256_xor_si256(_mm256_shuffle_epi8(t2_lo_lo, n1), _mm256_shuffle_epi8(t2_hi_lo, n2)),
            _mm256_xor_si256(_mm256_shuffle_epi8(t2_ulo_lo, n3), _mm256_shuffle_epi8(t2_uhi_lo, n4)),
        );
        let r2_hi = _mm256_xor_si256(
            _mm256_xor_si256(_mm256_shuffle_epi8(t2_lo_hi, n1), _mm256_shuffle_epi8(t2_hi_hi, n2)),
            _mm256_xor_si256(_mm256_shuffle_epi8(t2_ulo_hi, n3), _mm256_shuffle_epi8(t2_uhi_hi, n4)),
        );
        acc = _mm256_xor_si256(acc, _mm256_unpacklo_epi8(r2_lo, r2_hi));

        // Store once
        _mm256_storeu_si256(dst[off..].as_mut_ptr() as *mut __m256i, acc);
    }

    let rem = chunks * 32;
    if rem < len {
        mul_add_buffer_scalar(&mut dst[rem..], &src1[rem..], c1);
        mul_add_buffer_scalar(&mut dst[rem..], &src2[rem..], c2);
    }
}

/// Old batched multi-source (not used, replaced by pair batching above).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(dead_code)]
unsafe fn mul_add_multi_avx2(dst: &mut [u8], srcs: &[&[u8]], active: &[(usize, u16)]) {
    use std::arch::x86_64::*;

    let nibble_mask = _mm256_set1_epi8(0x0F);
    let deint_lo = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        0,2,4,6,8,10,12,14, -1,-1,-1,-1,-1,-1,-1,-1));
    let deint_hi = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        1,3,5,7,9,11,13,15, -1,-1,-1,-1,-1,-1,-1,-1));

    // Precompute all tables
    let all_tables: Vec<GfMulTables> = active.iter().map(|&(_, c)| GfMulTables::new(c)).collect();

    let len = dst.len();
    let chunks = len / 32;

    for chunk in 0..chunks {
        let off = chunk * 32;

        // Load dst once per 32-byte chunk
        let mut acc = _mm256_loadu_si256(dst[off..].as_ptr() as *const __m256i);

        // Accumulate all sources into this chunk
        for (src_i, &(src_idx, _)) in active.iter().enumerate() {
            let tables = &all_tables[src_i];
            let src = srcs[src_idx];

            let tbl_lo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.lo_lo.as_ptr() as *const _));
            let tbl_lo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.lo_hi.as_ptr() as *const _));
            let tbl_hi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.hi_lo.as_ptr() as *const _));
            let tbl_hi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.hi_hi.as_ptr() as *const _));
            let tbl_ulo_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.ulo_lo.as_ptr() as *const _));
            let tbl_ulo_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.ulo_hi.as_ptr() as *const _));
            let tbl_uhi_lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.uhi_lo.as_ptr() as *const _));
            let tbl_uhi_hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(tables.uhi_hi.as_ptr() as *const _));

            let src_data = _mm256_loadu_si256(src[off..].as_ptr() as *const __m256i);

            let src_lo_bytes = _mm256_shuffle_epi8(src_data, deint_lo);
            let src_hi_bytes = _mm256_shuffle_epi8(src_data, deint_hi);

            let lo_nib = _mm256_and_si256(src_lo_bytes, nibble_mask);
            let hi_nib = _mm256_and_si256(_mm256_srli_epi16(src_lo_bytes, 4), nibble_mask);
            let ulo_nib = _mm256_and_si256(src_hi_bytes, nibble_mask);
            let uhi_nib = _mm256_and_si256(_mm256_srli_epi16(src_hi_bytes, 4), nibble_mask);

            let r_lo = _mm256_xor_si256(
                _mm256_xor_si256(_mm256_shuffle_epi8(tbl_lo_lo, lo_nib), _mm256_shuffle_epi8(tbl_hi_lo, hi_nib)),
                _mm256_xor_si256(_mm256_shuffle_epi8(tbl_ulo_lo, ulo_nib), _mm256_shuffle_epi8(tbl_uhi_lo, uhi_nib)),
            );
            let r_hi = _mm256_xor_si256(
                _mm256_xor_si256(_mm256_shuffle_epi8(tbl_lo_hi, lo_nib), _mm256_shuffle_epi8(tbl_hi_hi, hi_nib)),
                _mm256_xor_si256(_mm256_shuffle_epi8(tbl_ulo_hi, ulo_nib), _mm256_shuffle_epi8(tbl_uhi_hi, uhi_nib)),
            );

            let result = _mm256_unpacklo_epi8(r_lo, r_hi);
            acc = _mm256_xor_si256(acc, result);
        }

        // Store accumulated result once
        _mm256_storeu_si256(dst[off..].as_mut_ptr() as *mut __m256i, acc);
    }

    // Scalar remainder
    let rem = chunks * 32;
    if rem < len {
        for &(src_idx, coeff) in active {
            mul_add_buffer_scalar(&mut dst[rem..], &srcs[src_idx][rem..], coeff);
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn xor_buffers_avx2(dst: &mut [u8], src: &[u8]) {
    use std::arch::x86_64::*;
    let len = dst.len();
    let chunks = len / 32;
    for chunk in 0..chunks {
        let off = chunk * 32;
        let s = _mm256_loadu_si256(src[off..].as_ptr() as *const __m256i);
        let d = _mm256_loadu_si256(dst[off..].as_ptr() as *const __m256i);
        _mm256_storeu_si256(dst[off..].as_mut_ptr() as *mut __m256i, _mm256_xor_si256(d, s));
    }
    let rem = chunks * 32;
    for i in rem..len { dst[i] ^= src[i]; }
}

// =========================================================================
// SSSE3 fallback (128-bit)
// =========================================================================

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn mul_add_buffer_ssse3(dst: &mut [u8], src: &[u8], constant: u16) {
    use std::arch::x86_64::*;

    let tables = GfMulTables::new(constant);
    let nibble_mask = _mm_set1_epi8(0x0F);

    let tbl_lo_lo = _mm_loadu_si128(tables.lo_lo.as_ptr() as *const __m128i);
    let tbl_lo_hi = _mm_loadu_si128(tables.lo_hi.as_ptr() as *const __m128i);
    let tbl_hi_lo = _mm_loadu_si128(tables.hi_lo.as_ptr() as *const __m128i);
    let tbl_hi_hi = _mm_loadu_si128(tables.hi_hi.as_ptr() as *const __m128i);
    let tbl_ulo_lo = _mm_loadu_si128(tables.ulo_lo.as_ptr() as *const __m128i);
    let tbl_ulo_hi = _mm_loadu_si128(tables.ulo_hi.as_ptr() as *const __m128i);
    let tbl_uhi_lo = _mm_loadu_si128(tables.uhi_lo.as_ptr() as *const __m128i);
    let tbl_uhi_hi = _mm_loadu_si128(tables.uhi_hi.as_ptr() as *const __m128i);

    let deint_lo = _mm_setr_epi8(0,2,4,6,8,10,12,14, -1,-1,-1,-1,-1,-1,-1,-1);
    let deint_hi = _mm_setr_epi8(1,3,5,7,9,11,13,15, -1,-1,-1,-1,-1,-1,-1,-1);

    let len = dst.len();
    let chunks = len / 16;

    for chunk in 0..chunks {
        let off = chunk * 16;
        let src_data = _mm_loadu_si128(src[off..].as_ptr() as *const __m128i);

        let src_lo_bytes = _mm_shuffle_epi8(src_data, deint_lo);
        let src_hi_bytes = _mm_shuffle_epi8(src_data, deint_hi);

        let lo_nib = _mm_and_si128(src_lo_bytes, nibble_mask);
        let hi_nib = _mm_and_si128(_mm_srli_epi16(src_lo_bytes, 4), nibble_mask);
        let ulo_nib = _mm_and_si128(src_hi_bytes, nibble_mask);
        let uhi_nib = _mm_and_si128(_mm_srli_epi16(src_hi_bytes, 4), nibble_mask);

        let r_lo = _mm_xor_si128(
            _mm_xor_si128(_mm_shuffle_epi8(tbl_lo_lo, lo_nib), _mm_shuffle_epi8(tbl_hi_lo, hi_nib)),
            _mm_xor_si128(_mm_shuffle_epi8(tbl_ulo_lo, ulo_nib), _mm_shuffle_epi8(tbl_uhi_lo, uhi_nib)),
        );
        let r_hi = _mm_xor_si128(
            _mm_xor_si128(_mm_shuffle_epi8(tbl_lo_hi, lo_nib), _mm_shuffle_epi8(tbl_hi_hi, hi_nib)),
            _mm_xor_si128(_mm_shuffle_epi8(tbl_ulo_hi, ulo_nib), _mm_shuffle_epi8(tbl_uhi_hi, uhi_nib)),
        );

        let result = _mm_unpacklo_epi8(r_lo, r_hi);
        let dst_val = _mm_loadu_si128(dst[off..].as_ptr() as *const __m128i);
        _mm_storeu_si128(dst[off..].as_mut_ptr() as *mut __m128i, _mm_xor_si128(dst_val, result));
    }

    let rem = chunks * 16;
    if rem < len {
        mul_add_buffer_scalar(&mut dst[rem..], &src[rem..], constant);
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mul_add_buffer_scalar_basic() {
        let src = [3u8, 0];
        let mut dst = [0u8, 0];
        mul_add_buffer(&mut dst, &src, 5);
        let expected = gf::mul(3, 5);
        let result = u16::from_le_bytes([dst[0], dst[1]]);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_mul_add_buffer_accumulates() {
        let src = [7u8, 0, 11, 0];
        let mut dst = [0xFFu8, 0x00, 0x00, 0x01];
        let constant = 42u16;
        mul_add_buffer(&mut dst, &src, constant);
        let expected0 = 0x00FF ^ gf::mul(constant, 7);
        let expected1 = 0x0100 ^ gf::mul(constant, 11);
        assert_eq!(u16::from_le_bytes([dst[0], dst[1]]), expected0);
        assert_eq!(u16::from_le_bytes([dst[2], dst[3]]), expected1);
    }

    #[test]
    fn test_mul_add_buffer_large() {
        let n = 4096; // Large enough for AVX2 path (>32 bytes)
        let mut src = vec![0u8; n];
        let mut dst_ref = vec![0u8; n];
        let mut dst_simd = vec![0u8; n];
        let constant = 12345u16;

        for i in 0..n / 2 {
            let val = (i as u16).wrapping_mul(7).wrapping_add(13);
            src[i * 2] = val as u8;
            src[i * 2 + 1] = (val >> 8) as u8;
        }

        mul_add_buffer_scalar(&mut dst_ref, &src, constant);
        mul_add_buffer(&mut dst_simd, &src, constant);
        assert_eq!(dst_simd, dst_ref, "SIMD and scalar results must match");
    }

    #[test]
    fn test_mul_add_multi_matches_sequential() {
        let n = 2048;
        let src1: Vec<u8> = (0..n).map(|i| (i * 3) as u8).collect();
        let src2: Vec<u8> = (0..n).map(|i| (i * 7 + 1) as u8).collect();
        let src3: Vec<u8> = (0..n).map(|i| (i * 11 + 5) as u8).collect();
        let coeffs = [100u16, 200, 300];
        let srcs: Vec<&[u8]> = vec![&src1, &src2, &src3];

        // Sequential reference
        let mut dst_seq = vec![0u8; n];
        mul_add_buffer(&mut dst_seq, &src1, 100);
        mul_add_buffer(&mut dst_seq, &src2, 200);
        mul_add_buffer(&mut dst_seq, &src3, 300);

        // Batched
        let mut dst_batch = vec![0u8; n];
        mul_add_multi(&mut dst_batch, &srcs, &coeffs);

        assert_eq!(dst_batch, dst_seq, "Batched multi-source must match sequential");
    }

    #[test]
    fn test_xor_buffers() {
        let src = vec![0xAAu8; 128];
        let mut dst = vec![0x55u8; 128];
        xor_buffers(&mut dst, &src);
        assert!(dst.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn test_mul_by_zero() {
        let src = vec![0xFF; 64];
        let mut dst = vec![0x00; 64];
        mul_add_buffer(&mut dst, &src, 0);
        assert!(dst.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_mul_by_one() {
        let src = vec![42u8, 0, 99, 0];
        let mut dst = vec![0u8; 4];
        mul_add_buffer(&mut dst, &src, 1);
        assert_eq!(dst, src);
    }
}
