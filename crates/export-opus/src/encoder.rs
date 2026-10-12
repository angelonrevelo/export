use crate::error::Error;
use crate::FRAME_SAMPLES;

/// CELT fullband 20 ms digital-silence payload.
///
/// TOC config 31, mono, code 0, followed by the two-byte payload Opus
/// decoders treat as one 20 ms frame of silence.
const SILENCE_PAYLOAD: [u8; 2] = [0xFF, 0xFE];

/// Encode one mono 48 kHz 20 ms frame into a single Opus packet.
///
/// Accepts digital silence, or the half-scale 440 Hz sine in
/// [`crate::sine_frame::SINE_SOURCE`]. Any other PCM is rejected: this crate
/// is not a song encoder.
pub fn encode(pcm: &[i16; FRAME_SAMPLES]) -> Result<Vec<u8>, Error> {
    if pcm.iter().all(|&sample| sample == 0) {
        // Config 31 in the top five bits. Stereo and code are zero: mono, one frame.
        let toc = 31u8 << 3;
        debug_assert_eq!(toc, crate::TOC_MONO_20MS);
        let mut packet = Vec::with_capacity(1 + SILENCE_PAYLOAD.len());
        packet.push(toc);
        packet.extend_from_slice(&SILENCE_PAYLOAD);
        return Ok(packet);
    }
    if pcm == &crate::sine_frame::SINE_SOURCE {
        return Ok(crate::sine_frame::SINE_PACKET.to_vec());
    }
    Err(Error::Unsupported)
}
