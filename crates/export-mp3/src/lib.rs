//! One MPEG-1 Layer III frame, mono, 44.1 kHz.
//!
//! [`encode_frame`] writes a single frame. [`decode_frame`] reads that frame
//! back to 1152 PCM samples. A non-silent block stores the first 32 quantized
//! long-block MDCT lines of each granule (indices 0 through 31), not a
//! waveform. There is no bit
//! reservoir. The library does not call ffmpeg.

#![forbid(unsafe_code)]

mod decoder;
mod encoder;
mod host;
mod window;

pub use decoder::{decode_frame, mpeg1_layer3_frame_bytes, Error};
pub use encoder::{
    encode_frame, half_scale_440hz, BITRATE_BPS, FRAME_BYTES, FRAME_SAMPLES, SAMPLE_RATE_HZ,
};
pub use host::{append_ffmpeg_demuxer_suffix, FFMPEG_DEMUXER_SUFFIX_BYTES};

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn demuxer_suffix_is_four_sync_bytes() {
        let frame = encode_frame(&half_scale_440hz()).unwrap();
        let openable = append_ffmpeg_demuxer_suffix(&frame).unwrap();
        assert_eq!(openable.len(), FRAME_BYTES + FFMPEG_DEMUXER_SUFFIX_BYTES);
        assert_eq!(&openable[FRAME_BYTES..], &frame[..4]);
    }

    #[test]
    fn demuxer_suffix_does_not_change_decode() {
        let frame = encode_frame(&half_scale_440hz()).unwrap();
        let openable = append_ffmpeg_demuxer_suffix(&frame).unwrap();
        let raw = decode_frame(&frame).unwrap();
        let from_openable = decode_frame(&openable).unwrap();
        assert_eq!(raw, from_openable);
    }
}
