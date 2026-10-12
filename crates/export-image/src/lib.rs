//! PNG decode, measured JPEG shrink-to-bytes, and simple WebP.
//!
//! Nothing in this crate calls Pillow, ImageMagick, ffmpeg, rembg, or libwebp.
//! Inflate, PNG filters, the baseline JPEG Huffman/DCT path, VP8L, the VP8
//! keyframe subset, and the resampler are local. There is no dependency.
//!
//! WebP is a still frame, or a two-frame or four-frame animation whose frames
//! are the same lossy VP8 keyframes: profile 0, DC prediction, loop filter off.
//! A later animation frame may be a smaller rectangle at an even pixel origin.
//! The decoder paints that rectangle onto the canvas. [`decode_webp`] returns
//! that composited canvas for two 64×64 partial files: the second frame 16×16
//! at (16, 32), and the second frame 16×16 at the origin. For the full-frame
//! 64×64 two-frame file, for the two-frame 256×256 file, for the two-frame
//! 640×480 file, and for the two-frame 1280×720 file, it returns the first
//! frame. Every other animation stays an
//! error on [`decode_webp`].
//! Alpha-only (`ALPH`) and other extended features stay errors.
//! Thumbnail and shrink do not take WebP. Not in this crate: HEIC, AVIF, TIFF,
//! SVG, RAW, JPEG XL, background removal, and vectorization. A file that
//! cannot be restored exactly is an error and is counted by
//! [`restore_lossless_all`].
//!
//! Shrink does not apply one quality to every image. It binary-searches
//! baseline JPEG quality, records each probe, and keeps the highest quality
//! whose encoded size fits the cap. A source already at or under the cap is
//! copied. The content hash is SHA-256, computed once per call, and a repeat
//! of the same bytes and cap returns the cached encode without running it again.
//!
//! [`thumbnail_jpeg`] is a separate path: the long edge is at most 220 pixels
//! and the JPEG quality is 72. Buffers larger than 50 MiB are rejected before
//! decode. Progressive, arithmetic, and unknown JPEG markers stay errors.

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
mod thumb;
mod vp8;
mod vp8_tab;
mod vp8l;
mod webp;

pub use error::{Error, ErrorCode};
pub use jpeg::{decode_jpeg, encode_jpeg};
pub use png::{decode_png, encode_png, restore_lossless, restore_lossless_all, LosslessReport};
pub use raster::Raster;
pub use resize::{resize, Filter};
pub use sha256::{sha256, Sha256};
pub use shrink::{shrink_png, Probe, ShrinkCache, ShrinkOutcome};
pub use sniff::{sniff, Format};
pub use thumb::{thumbnail_jpeg, Thumbnail, THUMB_LONG_EDGE, THUMB_QUALITY, THUMB_READ_LIMIT};
pub use webp::{decode_webp, decode_webp_animation, encode_webp_animation, encode_webp_lossless, encode_webp_lossy};

/// Decompressed pixel cap. Matches the product limit of 64 million pixels.
pub const MAX_PIXELS: u64 = 64_000_000;

/// Upper bound of the shrink search. Not a quality that is applied on its own.
pub const SHRINK_QUALITY_MAX: u8 = 95;
