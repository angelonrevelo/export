//! Host-only helpers. The library does not spawn or link ffmpeg.

use crate::decoder::Error;

/// Bytes appended after one frame so ffmpeg 6.1's MP3 demuxer can read four
/// header bytes at `frame_size`. This is not a second Layer III frame.
pub const FFMPEG_DEMUXER_SUFFIX_BYTES: usize = 4;

/// Append the demuxer suffix after `frame`.
///
/// The suffix repeats this frame's first four bytes (its MPEG sync and header
/// prefix). [`crate::decode_frame`] still reads only the Layer III frame when
/// the buffer is longer; pass `&openable[..crate::FRAME_BYTES]` if you want an
/// exact-length decode.
pub fn append_ffmpeg_demuxer_suffix(frame: &[u8]) -> Result<Vec<u8>, Error> {
    if frame.len() < 4 {
        return Err(Error::Truncated);
    }
    let mut out = frame.to_vec();
    out.extend_from_slice(&frame[..FFMPEG_DEMUXER_SUFFIX_BYTES]);
    Ok(out)
}

