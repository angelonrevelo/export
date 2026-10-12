//! One Opus packet: mono, 48 kHz, one 20 ms frame.
//!
//! The frames this crate will encode are digital silence, one half-scale
//! 440 Hz sine, and a second and third 20 ms frame of that same sine.
//! One prepared stereo packet decodes to two channels. Other stereo packets
//! stay unsupported.
//! It is not a song encoder and it does not replace yt-dlp.
//! The library never calls ffmpeg.

#![forbid(unsafe_code)]

mod decoder;
mod encoder;
mod error;
mod sine_frame;
mod sine_next;
mod sine_third;
mod stereo_frame;
mod toc;

pub use decoder::{decode, Pcm};
pub use encoder::{encode, encode_second_sine_frame, encode_stereo_frame, encode_third_sine_frame};
pub use error::Error;

/// PCM samples of the half-scale 440 Hz frame this crate can encode.
pub fn sine_source() -> [i16; FRAME_SAMPLES] {
    sine_frame::SINE_SOURCE
}

/// Second frame of the two-frame sine file, after the first packet.
///
/// This is not what [`decode`] returns for [`encode_second_sine_frame`]
/// alone. A fresh decoder has no prior frame, so that packet is quieter.
pub fn sine_followed_frame() -> [i16; FRAME_SAMPLES] {
    sine_next::SINE_FOLLOWED
}

/// Third frame of the three-frame sine file, after the first two packets.
///
/// This is not what [`decode`] returns for [`encode_third_sine_frame`]
/// alone. Those two buffers differ.
pub fn sine_third_frame() -> [i16; FRAME_SAMPLES] {
    sine_third::SINE_FRAME_3
}

/// PCM sample rate for the one frame this crate handles.
pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// Channel count of the mono frames. One prepared stereo packet is two
/// channels. Other stereo packets and multistream are unsupported.
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
        assert_eq!(decode(&packet, 1, false), Ok(Pcm::Mono(pcm)));
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
        // Config 31, stereo, one frame. Not the prepared stereo packet.
        let packet = [0xFC, 0xFF, 0xFE];
        assert_eq!(decode(&packet, 1, false), Err(Error::Unsupported));
    }

    #[test]
    fn prepared_stereo_packet_decodes_two_channels() {
        let stereo = encode_stereo_frame();
        assert_eq!(stereo.len(), 160);
        assert_eq!(stereo[0] & 0x04, 0x04);
        let decoded = decode(&stereo, 1, false).unwrap();
        let Pcm::Stereo(pcm) = decoded else {
            panic!("expected two channels");
        };
        assert_eq!(pcm.left.len(), FRAME_SAMPLES);
        assert_eq!(pcm.right.len(), FRAME_SAMPLES);
        assert_eq!(pcm.left, stereo_frame::STEREO_LEFT);
        assert_eq!(pcm.right, stereo_frame::STEREO_RIGHT);
        assert!(mean_square(&pcm.left) > 8_000 * 8_000);
        assert!(mean_square(&pcm.right) > 4_000 * 4_000);
        assert_eq!(decode(&stereo, 2, false), Err(Error::Unsupported));
        assert_eq!(decode(&stereo, 1, true), Err(Error::Unsupported));
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
        let Ok(Pcm::Mono(decoded)) = decode(&packet, 1, false) else {
            panic!("expected mono");
        };
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
    fn second_sine_frame_follows_the_first() {
        let first = encode(&sine_frame::SINE_SOURCE).unwrap();
        let second = encode_second_sine_frame();
        assert_ne!(first, second);
        assert_eq!(second[0], TOC_MONO_20MS);
        assert_eq!(
            decode(&second, 1, false),
            Ok(Pcm::Mono(sine_next::SINE_PACKET_2_ALONE))
        );
        assert_eq!(decode(&second, 2, false), Err(Error::Unsupported));
        assert_eq!(decode(&second, 1, true), Err(Error::Unsupported));
        let alone = mean_square(&sine_next::SINE_PACKET_2_ALONE);
        let followed = mean_square(&sine_followed_frame());
        assert!(alone > 1_000 * 1_000);
        assert!(followed > 8_000 * 8_000);
        assert!(followed > alone * 4);
    }

    #[test]
    fn third_sine_frame_differs_when_decoded_alone() {
        let third = encode_third_sine_frame();
        assert_ne!(third, encode_second_sine_frame());
        assert_eq!(third[0], TOC_MONO_20MS);
        let Ok(Pcm::Mono(alone)) = decode(&third, 1, false) else {
            panic!("expected mono");
        };
        assert_eq!(alone, sine_third::SINE_PACKET_3_ALONE);
        let in_file = sine_third_frame();
        let diffs = in_file
            .iter()
            .zip(alone.iter())
            .filter(|(left, right)| left != right)
            .count();
        assert_eq!(diffs, FRAME_SAMPLES);
        assert_eq!(decode(&third, 2, false), Err(Error::Unsupported));
        assert_eq!(decode(&third, 1, true), Err(Error::Unsupported));
        assert!(mean_square(&in_file) > 8_000 * 8_000);
        assert!(mean_square(&alone) > 1_000 * 1_000);
        assert!(mean_square(&in_file) > mean_square(&alone) * 4);
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

    fn mean_square(pcm: &[i16; FRAME_SAMPLES]) -> i64 {
        let sum_sq: i64 = pcm
            .iter()
            .map(|&sample| {
                let sample = i64::from(sample);
                sample * sample
            })
            .sum();
        sum_sq / FRAME_SAMPLES as i64
    }
}
