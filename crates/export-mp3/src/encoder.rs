//! MPEG-1 Layer III encoder for a single mono 44.1 kHz frame.
//!
//! The bitstream is one self-contained frame: no ID3 tag, no bit reservoir,
//! and no following frames. 1152 finite PCM samples become one Layer III
//! frame that [`crate::decode_frame`] turns back into PCM.
//!
//! Silence stays an empty Huffman region, so the decoder returns exact zeros.
//! Any other block is stored as one count-1 pulse per granule (values in
//! {-1, 0, 1}) scaled by `global_gain`. That is a legal Layer III payload,
//! not a model of the waveform and not a song encoder.

use crate::decoder::Error;

/// Samples in one MPEG-1 Layer III frame (2 granules × 576).
pub const FRAME_SAMPLES: usize = 1152;

/// MPEG-1 sampling rate this encoder writes.
pub const SAMPLE_RATE_HZ: u32 = 44_100;

/// Constant bitrate of the frame this encoder writes.
pub const BITRATE_BPS: u32 = 128_000;

/// Byte length of the frame this encoder writes (128 kbps, 44.1 kHz, no pad).
pub const FRAME_BYTES: usize = 417;

/// Side-information length for MPEG-1 mono (no CRC).
const SIDE_INFO_BYTES: usize = 17;

/// Bits for one table-A quadruple `(±1, 0, 0, 0)`, including its sign bit.
const PULSE_BITS: u32 = 5;

/// 1152 samples of `0.5 * sin(2π * 440 * n / 44100)`.
///
/// This is a reference signal for the one-frame path. Encoding it does not
/// reproduce the sine. See `max_abs_error` against [`crate::decode_frame`].
pub fn half_scale_440hz() -> [f32; FRAME_SAMPLES] {
    let mut pcm = [0.0; FRAME_SAMPLES];
    for (n, sample) in pcm.iter_mut().enumerate() {
        let t = n as f32 / SAMPLE_RATE_HZ as f32;
        *sample = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
    }
    pcm
}

/// Encode `pcm` as one MPEG-1 Layer III frame.
///
/// `pcm` must hold exactly [`FRAME_SAMPLES`] finite mono samples at
/// [`SAMPLE_RATE_HZ`]. The frame is mono, long-block, and carries no bit
/// reservoir (`main_data_begin = 0`).
///
/// This is not a song encoder. One call writes one frame and forgets it.
pub fn encode_frame(pcm: &[f32]) -> Result<Vec<u8>, Error> {
    if pcm.len() != FRAME_SAMPLES || pcm.iter().any(|sample| !sample.is_finite()) {
        return Err(Error::InvalidPcm);
    }

    let mut frame = vec![0u8; FRAME_BYTES];
    write_header(&mut frame[..4]);

    let peak = pcm.iter().fold(0.0f32, |max, sample| max.max(sample.abs()));
    if peak == 0.0 {
        let side = write_side_info(0, 0);
        frame[4..4 + SIDE_INFO_BYTES].copy_from_slice(&side);
        return Ok(frame);
    }

    let gain = gain_from_peak(peak);
    let sign = if pcm.iter().sum::<f32>() < 0.0 { 1 } else { 0 };
    let side = write_side_info(PULSE_BITS, gain);
    frame[4..4 + SIDE_INFO_BYTES].copy_from_slice(&side);

    // Two granules, each one quadruple. Table A code `0111` is (1,0,0,0);
    // the following bit is the sign (1 = negative).
    let mut main = BitWriter::new();
    for _granule in 0..2 {
        main.put(0b0111, 4);
        main.put(sign, 1);
    }
    let payload = main.finish();
    let start = 4 + SIDE_INFO_BYTES;
    frame[start..start + payload.len()].copy_from_slice(&payload);
    Ok(frame)
}

fn gain_from_peak(peak: f32) -> u32 {
    // |xr| = 2^((global_gain - 210) / 4) for a unit count-1 line.
    let scaled = 210.0 + 4.0 * peak.log2();
    if !scaled.is_finite() {
        return 0;
    }
    scaled.round().clamp(0.0, 255.0) as u32
}

fn write_header(out: &mut [u8]) {
    // sync(11)=1s, MPEG-1, Layer III, no CRC,
    // 128 kbps, 44.1 kHz, no padding, private=0,
    // mono, mode extension=0, not copyright, original, emphasis=none.
    out[0] = 0xFF;
    out[1] = 0xFB;
    out[2] = 0x90;
    out[3] = 0xC4;
}

/// MPEG-1 mono side information, two long blocks, no scale-factor bits.
///
/// `part2_3_length` is the Huffman budget of each granule. Zero means every
/// frequency line is zero (digital silence).
fn write_side_info(part2_3_length: u32, global_gain: u32) -> [u8; SIDE_INFO_BYTES] {
    let mut w = BitWriter::new();
    w.put(0, 9); // main_data_begin
    w.put(0, 5); // private_bits (mono)
    w.put(0, 4); // scfsi[0..4]
    for _granule in 0..2 {
        w.put(part2_3_length, 12);
        w.put(0, 9); // big_values
        w.put(global_gain, 8);
        w.put(0, 4); // scalefac_compress (slen1 = slen2 = 0)
        w.put(0, 1); // window_switching_flag (long block)
        w.put(0, 5); // table_select[0]
        w.put(0, 5); // table_select[1]
        w.put(0, 5); // table_select[2]
        w.put(0, 4); // region0_count
        w.put(0, 3); // region1_count
        w.put(0, 1); // preflag
        w.put(0, 1); // scalefac_scale
        w.put(0, 1); // count1table_select = table A
    }
    let bytes = w.finish();
    debug_assert_eq!(bytes.len(), SIDE_INFO_BYTES);
    let mut out = [0u8; SIDE_INFO_BYTES];
    out.copy_from_slice(&bytes);
    out
}

struct BitWriter {
    bytes: Vec<u8>,
    acc: u32,
    nbits: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    fn put(&mut self, value: u32, n: u8) {
        debug_assert!(n > 0 && n <= 24);
        debug_assert!(n == 32 || value < (1u32 << n));
        self.acc = (self.acc << n) | value;
        self.nbits += n;
        while self.nbits >= 8 {
            self.nbits -= 8;
            self.bytes.push((self.acc >> self.nbits) as u8);
            if self.nbits == 0 {
                self.acc = 0;
            } else {
                self.acc &= (1u32 << self.nbits) - 1;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.bytes.push((self.acc << (8 - self.nbits)) as u8);
        }
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_silence_as_empty_region() {
        let frame = encode_frame(&[0.0; FRAME_SAMPLES]).unwrap();
        assert_eq!(frame.len(), FRAME_BYTES);
        assert_eq!(&frame[..4], &[0xFF, 0xFB, 0x90, 0xC4]);
        assert!(frame[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn encodes_a_pulse_when_pcm_is_nonzero() {
        let mut pcm = [0.0f32; FRAME_SAMPLES];
        pcm[0] = 0.5;
        let frame = encode_frame(&pcm).unwrap();
        assert_eq!(frame.len(), FRAME_BYTES);
        assert_eq!(&frame[..4], &[0xFF, 0xFB, 0x90, 0xC4]);
        assert!(frame[4..].iter().any(|byte| *byte != 0));
    }

    #[test]
    fn rejects_bad_pcm() {
        assert_eq!(encode_frame(&[0.0; 10]), Err(Error::InvalidPcm));
        let mut nan = vec![0.0; FRAME_SAMPLES];
        nan[3] = f32::NAN;
        assert_eq!(encode_frame(&nan), Err(Error::InvalidPcm));
        nan[3] = f32::INFINITY;
        assert_eq!(encode_frame(&nan), Err(Error::InvalidPcm));
    }
}
