//! MPEG-1 Layer III encoder for a single mono 44.1 kHz frame.
//!
//! The bitstream is one self-contained frame: no ID3 tag, no bit reservoir,
//! and no following frames. 1152 finite PCM samples become one Layer III
//! frame that [`crate::decode_frame`] turns back into PCM.
//!
//! Silence stays an empty Huffman region, so the decoder returns exact zeros.
//! Any other block stores the first 32 quantized long-block MDCT lines of
//! each granule (indices 0 through 31, Huffman table 5, magnitudes 0..=3).
//! That is not a waveform encoder and not a song encoder.

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

/// Long-block MDCT lines stored per granule (indices 0 through 31).
/// The rest of the 576 lines are zero.
const MDCT_LINES: usize = 32;

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
        let side = write_side_info(&[GranuleSide::silence(), GranuleSide::silence()]);
        frame[4..4 + SIDE_INFO_BYTES].copy_from_slice(&side);
        return Ok(frame);
    }

    let lines = mdct_lines(pcm);
    let mut main = BitWriter::new();
    let mut granules = [GranuleSide::silence(), GranuleSide::silence()];
    for (granule, coeffs) in lines.iter().enumerate() {
        let (gain, quant) = quantize_lines(coeffs);
        let mut bits = 0u32;
        for pair in 0..(MDCT_LINES / 2) {
            let x = quant[pair * 2];
            let y = quant[pair * 2 + 1];
            bits += pair_bits(x, y);
            emit_pair(&mut main, x, y);
        }
        granules[granule] = GranuleSide::mdct(bits, gain);
    }
    let side = write_side_info(&granules);
    frame[4..4 + SIDE_INFO_BYTES].copy_from_slice(&side);
    let payload = main.finish();
    let start = 4 + SIDE_INFO_BYTES;
    frame[start..start + payload.len()].copy_from_slice(&payload);
    Ok(frame)
}

/// Long-block MDCT lines 0 through 31, one row per granule.
///
/// Lines 0..=17 are subband 0. Lines 18..=31 are the first 14 bins of
/// subband 1. These are the MDCT coefficients themselves. The decoder still
/// runs alias reduction across the subband boundary.
fn mdct_lines(pcm: &[f32]) -> [[f32; MDCT_LINES]; 2] {
    let mut analysis = Analysis::new();
    let mut history = [[0.0f32; 18]; 2];
    let mut out = [[0.0f32; MDCT_LINES]; 2];
    for (granule, dest) in out.iter_mut().enumerate() {
        let mut blocks = [[0.0f32; 36]; 2];
        for band in 0..2 {
            blocks[band][..18].copy_from_slice(&history[band]);
        }
        for slot in 0..18 {
            let start = (granule * 18 + slot) * 32;
            let bands = analysis.push_low_bands(&pcm[start..start + 32]);
            for band in 0..2 {
                blocks[band][18 + slot] = bands[band];
                history[band][slot] = bands[band];
            }
        }
        let subband0 = mdct18(&blocks[0]);
        let subband1 = mdct18(&blocks[1]);
        dest[..18].copy_from_slice(&subband0);
        dest[18..32].copy_from_slice(&subband1[..14]);
    }
    out
}

/// Polyphase analysis, 512-point delay, returning subbands 0 and 1.
struct Analysis {
    x: [f32; 512],
    off: usize,
}

impl Analysis {
    fn new() -> Self {
        Self {
            x: [0.0; 512],
            off: 0,
        }
    }

    fn push_low_bands(&mut self, samples: &[f32]) -> [f32; 2] {
        for (i, sample) in samples.iter().enumerate() {
            self.x[(31 - i + self.off) & 511] = *sample;
        }
        let mut z = [0.0f32; 512];
        for (i, bin) in z.iter_mut().enumerate() {
            *bin = self.x[(i + self.off) & 511] * crate::window::ANALYSIS_WINDOW[i];
        }
        self.off = (self.off + 480) & 511;
        let mut y = [0.0f32; 64];
        for (i, bin) in y.iter_mut().enumerate() {
            let mut sum = 0.0f32;
            for group in 0..8 {
                sum += z[i + 64 * group];
            }
            *bin = sum;
        }
        let mut bands = [0.0f32; 2];
        for (band, dest) in bands.iter_mut().enumerate() {
            let factor = (2 * band + 1) as f32;
            for (k, bin) in y.iter().enumerate() {
                let angle = std::f32::consts::PI / 64.0 * factor * (16 - k as i32) as f32;
                *dest += *bin * angle.cos();
            }
        }
        bands
    }
}

/// Forward MDCT that inverts this crate's 36-point IMDCT, scale `1/9`.
fn mdct18(block: &[f32; 36]) -> [f32; 18] {
    let mut out = [0.0f32; 18];
    for (k, coeff) in out.iter_mut().enumerate() {
        let mut sum = 0.0f32;
        for (i, sample) in block.iter().enumerate() {
            let window = (std::f32::consts::PI / 36.0 * (i as f32 + 0.5)).sin();
            let angle = std::f32::consts::PI / 72.0 * (2 * i + 19) as f32 * (2 * k + 1) as f32;
            sum += window * *sample * angle.cos();
        }
        *coeff = sum / 9.0;
    }
    out
}

/// Quantize to Huffman table 5 (magnitudes 0..=3).
///
/// `global_gain` is chosen so the largest rounded magnitude is at most 3,
/// and as large as that limit allows.
fn quantize_lines(xr: &[f32; MDCT_LINES]) -> (u32, [i32; MDCT_LINES]) {
    let max_abs = xr.iter().fold(0.0f32, |max, bin| max.max(bin.abs()));
    if max_abs == 0.0 {
        return (0, [0; MDCT_LINES]);
    }
    let unit = 3.0f32.powf(4.0 / 3.0);
    let guess = 210.0 + 4.0 * (max_abs / unit).log2();
    let mut gain = if guess.is_finite() {
        guess.round().clamp(0.0, 255.0) as i32
    } else {
        0
    };
    while gain < 255 && peak_mag(xr, gain) > 3 {
        gain += 1;
    }
    while gain > 0 {
        let lower = peak_mag(xr, gain - 1);
        let current = peak_mag(xr, gain);
        if lower <= 3 && lower > current {
            gain -= 1;
        } else {
            break;
        }
    }
    (gain as u32, quantize_at(xr, gain))
}

fn peak_mag(xr: &[f32; MDCT_LINES], gain: i32) -> i32 {
    xr.iter()
        .fold(0, |max, bin| max.max(raw_mag(bin.abs(), gain)))
}

fn raw_mag(abs_xr: f32, gain: i32) -> i32 {
    if abs_xr == 0.0 {
        return 0;
    }
    let scale = 2.0f32.powf((gain - 210) as f32 * 0.25);
    if scale == 0.0 {
        return 4;
    }
    let mag = (abs_xr / scale).powf(0.75);
    if !mag.is_finite() || mag >= 4.0 {
        return 4;
    }
    mag.round() as i32
}

fn quantize_at(xr: &[f32; MDCT_LINES], gain: i32) -> [i32; MDCT_LINES] {
    let mut out = [0i32; MDCT_LINES];
    for (slot, bin) in out.iter_mut().zip(xr.iter()) {
        let mag = raw_mag(bin.abs(), gain).min(3);
        *slot = if *bin < 0.0 { -mag } else { mag };
    }
    out
}

/// Table 5 codewords, indexed by `|x| * 4 + |y|`. `(length, code)`, MSB first.
const TABLE5: [(u8, u32); 16] = [
    (1, 0b1),         // (0, 0)
    (3, 0b010),       // (0, 1)
    (6, 0b000_110),   // (0, 2)
    (7, 0b000_0101),  // (0, 3)
    (3, 0b011),       // (1, 0)
    (3, 0b001),       // (1, 1)
    (6, 0b000_100),   // (1, 2)
    (7, 0b000_0100),  // (1, 3)
    (6, 0b000_111),   // (2, 0)
    (6, 0b000_101),   // (2, 1)
    (7, 0b000_0111),  // (2, 2)
    (8, 0b0000_0001), // (2, 3)
    (7, 0b000_0110),  // (3, 0)
    (6, 0b000_001),   // (3, 1)
    (7, 0b000_0001),  // (3, 2)
    (8, 0b0000_0000), // (3, 3)
];

fn table5_entry(x: i32, y: i32) -> (u8, u32) {
    TABLE5[x.unsigned_abs() as usize * 4 + y.unsigned_abs() as usize]
}

fn pair_bits(x: i32, y: i32) -> u32 {
    let (len, _) = table5_entry(x, y);
    u32::from(len) + u32::from(x != 0) + u32::from(y != 0)
}

fn emit_pair(writer: &mut BitWriter, x: i32, y: i32) {
    let (len, code) = table5_entry(x, y);
    writer.put(code, len);
    if x != 0 {
        writer.put(u32::from(x < 0), 1);
    }
    if y != 0 {
        writer.put(u32::from(y < 0), 1);
    }
}

struct GranuleSide {
    part2_3_length: u32,
    big_values: u32,
    global_gain: u32,
    table_select_0: u32,
    region0_count: u32,
}

impl GranuleSide {
    fn silence() -> Self {
        Self {
            part2_3_length: 0,
            big_values: 0,
            global_gain: 0,
            table_select_0: 0,
            region0_count: 0,
        }
    }

    /// Sixteen table-5 pairs. Region 0 ends at line 36 (`region0_count = 7`),
    /// which covers spectral indices 0 through 31.
    fn mdct(part2_3_length: u32, global_gain: u32) -> Self {
        Self {
            part2_3_length,
            big_values: (MDCT_LINES / 2) as u32,
            global_gain,
            table_select_0: 5,
            region0_count: 7,
        }
    }
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
/// A silent granule has `part2_3_length = 0` and `big_values = 0`. A signal
/// granule has `big_values = 16`, Huffman table 5 in region 0, and
/// `region0_count = 7` so that region ends at spectral line 36.
fn write_side_info(granules: &[GranuleSide; 2]) -> [u8; SIDE_INFO_BYTES] {
    let mut w = BitWriter::new();
    w.put(0, 9); // main_data_begin
    w.put(0, 5); // private_bits (mono)
    w.put(0, 4); // scfsi[0..4]
    for granule in granules {
        w.put(granule.part2_3_length, 12);
        w.put(granule.big_values, 9);
        w.put(granule.global_gain, 8);
        w.put(0, 4); // scalefac_compress (slen1 = slen2 = 0)
        w.put(0, 1); // window_switching_flag (long block)
        w.put(granule.table_select_0, 5);
        w.put(0, 5); // table_select[1]
        w.put(0, 5); // table_select[2]
        w.put(granule.region0_count, 4);
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
    fn encodes_nonzero_pcm_with_side_info() {
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
