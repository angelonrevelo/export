//! ISO BMFF remux and WAV/PCM for Export.
//!
//! The product path is pure Rust. ffmpeg is not linked and is not spawned.
//! Each input is hashed once (SHA-256, streaming), sniffed by magic bytes,
//! and skipped when that digest was already ingested. Codecs run only after
//! that gate.
//!
//! Working jobs: stream-copy remux of non-fragmented mp4/mov/m4a, elementary
//! sample copy, and WAV/PCM read, write, downmix, and integer resample.
//! Fragmented MP4 (`moof`) is refused. AAC, Opus, MP3, FLAC, GIF, and WebM
//! are identified and not decoded.

mod bmff;
mod dedup;
mod error;
mod export;
mod pcm;
mod sha256;
mod sniff;
mod wav;

pub use bmff::{
    append_samples, build_movie, elementary, parse, remux, Movie, MovieSpec, RemuxOpt, SampleTable,
    Track, TrackSpec,
};
pub use dedup::{digest_of, Dedup, Prior};
pub use error::{Error, Result};
pub use export::{export, export_reader, Job, Output, DEFAULT_MAX_BYTES};
pub use sha256::{hash, hash_reader, hex, Sha256};
pub use sniff::{sniff, MediaKind};
pub use wav::{decode as decode_wav, encode as encode_wav, Wav, WavSpec};
