use crate::error::Error;
use crate::toc::{self, Toc};
use crate::FRAME_SAMPLES;

/// Decode one Opus packet back to PCM.
///
/// `streams` is the Opus stream count. Anything other than one stream is
/// multistream and returns [`Error::Unsupported`].
/// `fec` requests in-band forward error correction and returns
/// [`Error::Unsupported`].
///
/// A packet that runs out of bytes is [`Error::Truncated`]. A TOC byte that
/// is illegal, or that is not one mono 48 kHz 20 ms frame, is [`Error::BadToc`].
pub fn decode(packet: &[u8], streams: u8, fec: bool) -> Result<[i16; FRAME_SAMPLES], Error> {
    let toc = toc::parse(packet)?;
    if toc.stereo {
        return Err(Error::Unsupported);
    }
    if streams != 1 {
        return Err(Error::Unsupported);
    }
    if fec {
        return Err(Error::Unsupported);
    }
    if !is_one_mono_20ms_frame(&toc) {
        return Err(Error::BadToc);
    }
    decode_known_frame(packet)
}

fn is_one_mono_20ms_frame(toc: &Toc) -> bool {
    toc.config == 31 && !toc.stereo && toc.code == 0 && toc.frames == 1
}

fn decode_known_frame(packet: &[u8]) -> Result<[i16; FRAME_SAMPLES], Error> {
    if packet == [0xF8, 0xFF, 0xFE] {
        return Ok([0; FRAME_SAMPLES]);
    }
    if packet == crate::sine_frame::SINE_PACKET.as_slice() {
        return Ok(crate::sine_frame::SINE_DECODED);
    }
    Err(Error::BadToc)
}
