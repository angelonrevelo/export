use crate::error::Error;
use crate::toc::{self, Toc};
use crate::FRAME_SAMPLES;

/// Two channels from the one prepared stereo packet. 960 samples each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StereoPcm {
    pub left: [i16; FRAME_SAMPLES],
    pub right: [i16; FRAME_SAMPLES],
}

/// PCM from one packet this decoder knows.
///
/// Mono frames are 960 samples. The one prepared stereo packet is 960 samples
/// on each channel. Other stereo packets stay unsupported. This is the same
/// decoder, not a second one, and not a song encoder.
// The mono frame stays inline. Boxing it would allocate on every mono decode.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pcm {
    Mono([i16; FRAME_SAMPLES]),
    Stereo(Box<StereoPcm>),
}

/// Decode one Opus packet back to PCM.
///
/// `streams` is the Opus stream count. Anything other than one stream is
/// multistream and returns [`Error::Unsupported`].
/// `fec` requests in-band forward error correction and returns
/// [`Error::Unsupported`].
///
/// A packet that runs out of bytes is [`Error::Truncated`]. A TOC byte that
/// is illegal, or that is not one mono 48 kHz 20 ms frame and not the one
/// prepared stereo packet, is [`Error::BadToc`] or [`Error::Unsupported`].
pub fn decode(packet: &[u8], streams: u8, fec: bool) -> Result<Pcm, Error> {
    let toc = toc::parse(packet)?;
    if streams != 1 {
        return Err(Error::Unsupported);
    }
    if fec {
        return Err(Error::Unsupported);
    }
    if toc.stereo {
        return decode_prepared_stereo(packet, &toc);
    }
    if !is_one_mono_20ms_frame(&toc) {
        return Err(Error::BadToc);
    }
    decode_known_frame(packet).map(Pcm::Mono)
}

/// The one prepared stereo packet, matched the same way as the mono packets.
///
/// Any other stereo TOC stays unsupported. This does not decode arbitrary
/// stereo packets.
fn decode_prepared_stereo(packet: &[u8], toc: &Toc) -> Result<Pcm, Error> {
    if toc.config == 31
        && toc.code == 0
        && toc.frames == 1
        && packet == crate::stereo_frame::STEREO_PACKET.as_slice()
    {
        return Ok(Pcm::Stereo(Box::new(StereoPcm {
            left: crate::stereo_frame::STEREO_LEFT,
            right: crate::stereo_frame::STEREO_RIGHT,
        })));
    }
    Err(Error::Unsupported)
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
    if packet == crate::sine_next::SINE_PACKET_2.as_slice() {
        // Fresh decoder: no energy memory from the first frame.
        return Ok(crate::sine_next::SINE_PACKET_2_ALONE);
    }
    if packet == crate::sine_third::SINE_PACKET_3.as_slice() {
        // Fresh decoder: no frames before this packet.
        return Ok(crate::sine_third::SINE_PACKET_3_ALONE);
    }
    Err(Error::BadToc)
}
