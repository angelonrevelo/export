//! One Opus packet: mono, 48 kHz, one 20 ms frame.
//!
//! The frames this crate will encode are digital silence and one half-scale
//! 440 Hz sine. It is not a song encoder and it does not replace yt-dlp.
//! The library never calls ffmpeg.

#![forbid(unsafe_code)]

mod decoder;
mod encoder;
mod error;
mod sine_frame;
mod toc;

pub use decoder::decode;
pub use encoder::encode;
pub use error::Error;

/// PCM samples of the half-scale 440 Hz frame this crate can encode.
pub fn sine_source() -> [i16; FRAME_SAMPLES] {
    sine_frame::SINE_SOURCE
}

/// PCM sample rate for the one frame this crate handles.
pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// Channel count. Stereo and multistream are unsupported.
pub const CHANNELS: u8 = 1;

/// Frame duration in milliseconds.
pub const FRAME_MS: u32 = 20;

/// Samples in one mono 20 ms frame at 48 kHz.
pub const FRAME_SAMPLES: usize = 960;

/// TOC byte for CELT-only fullband, 20 ms, mono, one frame (config 31, code 0).
pub const TOC_MONO_20MS: u8 = 0xF8;

#[cfg(test)]
mod tests {
    use super::*;

    fn silence() -> [i16; FRAME_SAMPLES] {
        [0; FRAME_SAMPLES]
    }

    #[test]
    fn roundtrip_one_packet() {
        let pcm = silence();
        let packet = encode(&pcm).unwrap();
        assert_eq!(packet.first().copied(), Some(TOC_MONO_20MS));
        let decoded = decode(&packet, 1, false).unwrap();
        assert_eq!(decoded, pcm);
    }

    #[test]
    fn truncated_packet_is_an_error() {
        assert_eq!(decode(&[], 1, false), Err(Error::Truncated));
        assert_eq!(decode(&[TOC_MONO_20MS], 1, false), Err(Error::Truncated));
    }

    #[test]
    fn bad_toc_is_an_error() {
        // Config 0, mono, one frame: not the 48 kHz 20 ms TOC this crate accepts.
        assert_eq!(decode(&[0x00, 0x00], 1, false), Err(Error::BadToc));
        // Code 3 with a frame count of 0 is an illegal TOC structure.
        assert_eq!(decode(&[0xFB, 0x00], 1, false), Err(Error::BadToc));
    }

    #[test]
    fn stereo_is_unsupported() {
        // Config 31, stereo, one frame.
        let packet = [0xFC, 0xFF, 0xFE];
        assert_eq!(decode(&packet, 1, false), Err(Error::Unsupported));
    }

    #[test]
    fn multistream_is_unsupported() {
        let pcm = silence();
        let packet = encode(&pcm).unwrap();
        assert_eq!(decode(&packet, 2, false), Err(Error::Unsupported));
    }

    #[test]
    fn fec_is_unsupported() {
        let pcm = silence();
        let packet = encode(&pcm).unwrap();
        assert_eq!(decode(&packet, 1, true), Err(Error::Unsupported));
    }

    #[test]
    fn other_pcm_is_unsupported() {
        let mut pcm = silence();
        pcm[0] = 1;
        assert_eq!(encode(&pcm), Err(Error::Unsupported));
    }

    #[test]
    fn half_scale_sine_decodes_as_a_tone() {
        let packet = encode(&sine_frame::SINE_SOURCE).unwrap();
        assert_eq!(packet.as_slice(), sine_frame::SINE_PACKET.as_slice());
        assert_eq!(packet[0], TOC_MONO_20MS);
        let decoded = decode(&packet, 1, false).unwrap();
        assert_eq!(decoded, sine_frame::SINE_DECODED);
        let sum_sq: i64 = decoded
            .iter()
            .map(|&sample| {
                let sample = i64::from(sample);
                sample * sample
            })
            .sum();
        // RMS around 10_800. Silence is 0. This frame is not near silence.
        assert!(sum_sq / FRAME_SAMPLES as i64 > 8_000 * 8_000);
        // The window leaves the start of the frame quiet.
        assert!(decoded[0].unsigned_abs() < 10);
        assert!(decoded[400].unsigned_abs() > 5_000);
    }

    #[test]
    fn code3_missing_frame_byte_is_truncated() {
        // Config 31, mono, code 3, one frame, no frame byte.
        assert_eq!(decode(&[0xFB, 0x01], 1, false), Err(Error::Truncated));
    }

    #[test]
    fn padding_overrun_is_truncated() {
        // Code 3, one frame, padding length 10, and no frame or padding bytes.
        assert_eq!(decode(&[0xFB, 0x41, 10], 1, false), Err(Error::Truncated));
    }

    #[test]
    fn vbr_length_overrun_is_truncated() {
        // Code 3, VBR, two frames, first frame claims 50 bytes that are absent.
        assert_eq!(decode(&[0xFB, 0x82, 50], 1, false), Err(Error::Truncated));
    }
}
