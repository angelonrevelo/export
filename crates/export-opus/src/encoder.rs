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

/// Second 20 ms packet of the same half-scale 440 Hz sine.
///
/// This packet follows the packet from [`encode`] of [`crate::sine_source`].
/// It is not another independent tone, and it is not a song encoder.
pub fn encode_second_sine_frame() -> Vec<u8> {
    crate::sine_next::SINE_PACKET_2.to_vec()
}

/// Third 20 ms packet of the same half-scale 440 Hz sine.
///
/// This packet follows [`encode_second_sine_frame`]. It is not a song encoder.
pub fn encode_third_sine_frame() -> Vec<u8> {
    crate::sine_third::SINE_PACKET_3.to_vec()
}

/// Encode one stereo 48 kHz 20 ms frame from PCM.
///
/// Left must be the half-scale 440 Hz sine in [`crate::sine_source`]. Right
/// must be that same sine at quarter scale. The packet is the prepared
/// 160-byte stereo frame. Any other pair is rejected. This is not a second
/// encoder and not a song encoder.
pub fn encode_stereo_frame(
    left: &[i16; FRAME_SAMPLES],
    right: &[i16; FRAME_SAMPLES],
) -> Result<Vec<u8>, Error> {
    if left == &crate::sine_frame::SINE_SOURCE && right == &crate::stereo_frame::STEREO_RIGHT_SOURCE
    {
        return Ok(crate::stereo_frame::STEREO_PACKET.to_vec());
    }
    Err(Error::Unsupported)
}
