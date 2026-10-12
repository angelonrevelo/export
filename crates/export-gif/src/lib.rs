//! GIF87a and GIF89a LZW, with no dependencies.
//!
//! Ingest hashes the bytes once, checks the dedup map, and only then runs the
//! codec. Threading is off: this crate never spawns a thread. Video is sniffed
//! and refused. [`video_to_gif`] does not decode a container and does not call
//! ffmpeg.
//!
//! Not in this crate: a video decoder, palette quantization from full color,
//! and plain-text extensions as pixels. Interlaced frames are decoded into
//! row order and written back without the interlace flag.

#![forbid(unsafe_code)]

mod dedup;
mod error;
mod gif;
mod lzw;
mod sha256;
mod sniff;

pub use dedup::{read_gif, write_gif, Cache, ReadGif, WrittenGif};
pub use error::{Error, ErrorCode};
pub use gif::{decode_gif, encode_gif, video_to_gif, Frame, Gif, Version};
pub use sha256::{sha256, Sha256};
pub use sniff::{sniff, Format};

/// Decompressed pixel cap, matching the product limit of 64 million pixels.
pub const MAX_PIXELS: u64 = 64_000_000;

/// Embedded default. The codec is serial.
pub const THREADING: bool = false;
