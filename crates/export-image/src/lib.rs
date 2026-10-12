//! PNG decode and measured JPEG shrink-to-bytes.
//!
//! Nothing in this crate calls Pillow, ImageMagick, ffmpeg, or rembg. Inflate,
//! PNG filters, the baseline JPEG Huffman/DCT path, and the resampler are
//! local. There is no dependency.
//!
//! Not in this crate: WebP, HEIC, AVIF, TIFF, SVG, RAW, JPEG XL, background
//! removal, and vectorization. WebP and JPEG are sniffed so they are not
//! decoded as PNG. A file that cannot be restored exactly is an error and is
//! counted by [`restore_lossless_all`].
//!
//! Shrink does not apply one quality to every image. It binary-searches
//! baseline JPEG quality, records each probe, and keeps the highest quality
//! whose encoded size fits the cap. A source already at or under the cap is
//! copied. The content hash is SHA-256, computed once per call, and a repeat
//! of the same bytes and cap returns the cached encode without running it again.

#![forbid(unsafe_code)]

mod error;
mod inflate;
mod jpeg;
mod png;
mod raster;
mod resize;
mod sha256;
mod shrink;
mod sniff;

pub use error::{Error, ErrorCode};
pub use jpeg::{decode_jpeg, encode_jpeg};
pub use png::{decode_png, encode_png, restore_lossless, restore_lossless_all, LosslessReport};
pub use raster::Raster;
pub use resize::{resize, Filter};
pub use sha256::{sha256, Sha256};
pub use shrink::{shrink_png, Probe, ShrinkCache, ShrinkOutcome};
pub use sniff::{sniff, Format};

/// Decompressed pixel cap. Matches the product limit of 64 million pixels.
pub const MAX_PIXELS: u64 = 64_000_000;

/// Upper bound of the shrink search. Not a quality that is applied on its own.
pub const SHRINK_QUALITY_MAX: u8 = 95;
