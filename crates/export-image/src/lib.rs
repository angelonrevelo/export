//! PNG decode, measured JPEG shrink-to-bytes, and simple WebP.
//!
//! Nothing in this crate calls Pillow, ImageMagick, ffmpeg, rembg, libwebp,
//! Cairo, vtracer, or resvg. The SVG path does not start a thread.
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
//! 640×480 file, for the two-frame 1280×720 file, and for the four-frame
//! 1280×720 file, it returns the first frame. Every other animation stays an
//! error on [`decode_webp`].
//! Alpha-only (`ALPH`) and other extended features stay errors.
//! [`thumbnail_jpeg`] of the two-frame 640×480 WebP, and of the four-frame
//! 1280×720 WebP, writes a JPEG of that file's first frame, long edge 220,
//! quality 72. Every other WebP stays unsupported on that path. Shrink does not
//! take WebP. [`encode_svg`] writes one 16×16 or
//! 32×32 black square on white as a single `rect`, and [`rasterize_svg`] paints
//! that file back. A path, a curve, text, and any other picture stay unsupported.
//! Not in this crate: HEIC, AVIF, TIFF, RAW, JPEG XL, background removal, and
//! vectorization. A file that
//! cannot be restored exactly is an error and is counted by
//! [`restore_lossless_all`].
//!
//! Shrink does not apply one quality to every image. It binary-searches
//! baseline JPEG quality, records each probe, and keeps the highest quality
//! whose encoded size fits the cap. A source already at or under the cap is
//! copied. The content hash is SHA-256, computed once per call, and a repeat
//! of the same bytes and cap returns the cached encode without running it again.
//! When the standard Huffman tables still miss the cap after six scales, the
//! search starts over with tables counted from the image and with each side
//! truncated to 70 percent, at least 2 pixels. That second file is still a
//! baseline JPEG. The cap is not raised.
//!
//! [`thumbnail_jpeg`] is a separate path: the long edge is at most 220 pixels
//! and the JPEG quality is 72. The WebP inputs are the two-frame 640×480 file
//! and the four-frame 1280×720 file, and the picture is that file's first frame.
//! Buffers larger than 50 MiB are rejected before decode. Progressive,
//! arithmetic, and unknown JPEG markers stay errors.

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
mod svg;
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
pub use svg::{encode_svg, rasterize_svg};
pub use thumb::{thumbnail_jpeg, Thumbnail, THUMB_LONG_EDGE, THUMB_QUALITY, THUMB_READ_LIMIT};
pub use webp::{
    decode_webp, decode_webp_animation, encode_webp_animation, encode_webp_lossless,
    encode_webp_lossy,
};

/// Decompressed pixel cap. Matches the product limit of 64 million pixels.
pub const MAX_PIXELS: u64 = 64_000_000;

/// Upper bound of the shrink search. Not a quality that is applied on its own.
pub const SHRINK_QUALITY_MAX: u8 = 95;
