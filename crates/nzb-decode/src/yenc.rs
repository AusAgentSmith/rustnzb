//! yEnc decoder — delegates to the `yenc-simd` crate (SIMD-accelerated).

pub use yenc_simd::{YencDecodeResult, YencError, decode_yenc, encode_article};
