//! One Vorbis audio packet: mono, 48 kHz, one short frame.
//!
//! `encode_packet` writes a silent frame. `encode_half_scale_sine` writes one
//! half-scale 440 Hz frame. `decode_packet` reads either bitstream back to PCM.
//! This crate is not a song encoder.

#![forbid(unsafe_code)]

mod decoder;
mod dsp;
mod encoder;

pub use decoder::decode_packet;
pub use encoder::{encode_half_scale_sine, encode_packet, half_scale_sine};

/// Sample rate of the one packet this crate writes.
pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// PCM samples produced by the one short frame (half of a 64-sample block).
pub const FRAME_SAMPLES: usize = 32;

pub(crate) const BLOCKSIZE_EXP: u8 = 6;
pub(crate) const BLOCKSIZE: usize = 1 << BLOCKSIZE_EXP;

/// Failure while reading or writing the one Vorbis packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The packet ended before a required field.
    Truncated,
    /// A header magic, framing bit, or setup field is invalid.
    BadHeader,
    /// Stereo, a non-48 kHz rate, or a frame this crate does not write.
    Unsupported,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Truncated => write!(f, "truncated Vorbis packet"),
            Error::BadHeader => write!(f, "bad Vorbis header"),
            Error::Unsupported => write!(f, "unsupported Vorbis stream"),
        }
    }
}

impl std::error::Error for Error {}

const OGG_CRC_TABLE: [u32; 256] = make_ogg_crc_table();

const fn make_ogg_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            if crc & 0x8000_0000 != 0 {
                crc = (crc << 1) ^ 0x04C1_1DB7;
            } else {
                crc <<= 1;
            }
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

pub(crate) fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        let index = ((crc >> 24) ^ u32::from(byte)) as usize;
        crc = (crc << 8) ^ OGG_CRC_TABLE[index];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_frame_decodes_to_pcm() {
        let pcm = [0.0f32; FRAME_SAMPLES];
        let packet = encode_packet(&pcm).expect("encode");
        let decoded = decode_packet(&packet).expect("decode");
        assert_eq!(decoded.len(), FRAME_SAMPLES);
        assert!(decoded.iter().all(|sample| sample.abs() < 1.0e-6));
    }

    #[test]
    fn truncated_packet_errors() {
        let packet = encode_packet(&[0.0; FRAME_SAMPLES]).expect("encode");
        assert_eq!(decode_packet(&[]).unwrap_err(), Error::Truncated);
        assert_eq!(decode_packet(&packet[..20]).unwrap_err(), Error::Truncated);
        assert_eq!(
            decode_packet(&packet[..packet.len() - 1]).unwrap_err(),
            Error::Truncated
        );
    }

    #[test]
    fn bad_header_errors() {
        let mut packet = encode_packet(&[0.0; FRAME_SAMPLES]).expect("encode");
        packet[0] = b'X';
        assert_eq!(decode_packet(&packet).unwrap_err(), Error::BadHeader);
    }

    #[test]
    fn stereo_is_unsupported() {
        let mut packet = encode_packet(&[0.0; FRAME_SAMPLES]).expect("encode");
        // Identification channel byte sits 11 bytes into the first packet,
        // after a 28-byte Ogg page prefix (27 header bytes + one lace byte).
        let channel = 28 + 11;
        packet[channel] = 2;
        let crc = {
            let mut page_end = 27 + packet[26] as usize;
            let body: usize = packet[27..page_end].iter().map(|&v| v as usize).sum();
            page_end += body;
            packet[22..26].fill(0);
            let crc = ogg_crc(&packet[..page_end]);
            packet[22..26].copy_from_slice(&crc.to_le_bytes());
            crc
        };
        let _ = crc;
        assert_eq!(decode_packet(&packet).unwrap_err(), Error::Unsupported);
    }

    #[test]
    fn non_silent_frame_is_unsupported() {
        let mut pcm = [0.0f32; FRAME_SAMPLES];
        pcm[0] = 0.25;
        assert_eq!(encode_packet(&pcm).unwrap_err(), Error::Unsupported);
    }

    #[test]
    fn half_scale_sine_decodes() {
        let ideal = half_scale_sine();
        assert!(ideal[0].abs() < 1.0e-6);
        assert!(ideal.iter().any(|sample| sample.abs() > 0.2));

        let packet = encode_half_scale_sine().expect("encode");
        let decoded = decode_packet(&packet).expect("decode");
        assert_eq!(decoded.len(), FRAME_SAMPLES);
        let mut error = 0.0f32;
        let mut peak = 0.0f32;
        for (got, want) in decoded.iter().zip(ideal.iter()) {
            let delta = got - want;
            error += delta * delta;
            peak = peak.max(got.abs());
        }
        let rms = (error / FRAME_SAMPLES as f32).sqrt();
        assert!(peak > 0.2, "peak {peak}");
        assert!(rms < 0.02, "rms {rms}");
    }

    #[test]
    fn truncated_sine_errors() {
        let packet = encode_half_scale_sine().expect("encode");
        assert_eq!(decode_packet(&packet[..20]).unwrap_err(), Error::Truncated);
        assert_eq!(
            decode_packet(&packet[..packet.len() - 1]).unwrap_err(),
            Error::Truncated
        );
    }
}
