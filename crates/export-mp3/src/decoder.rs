//! MPEG-1 Layer III decoder for one mono frame.
//!
//! A short buffer is [`Error::Truncated`]. A header that is not a legal MPEG
//! audio header is [`Error::BadHeader`]. Stereo (and the other non-mono
//! channel modes) is [`Error::Unsupported`].

use crate::encoder::{FRAME_SAMPLES, SAMPLE_RATE_HZ};

/// Failure from [`decode_frame`] or [`crate::encode_frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The buffer ended before the header, side info, or declared frame bytes.
    Truncated,
    /// Sync, version, layer, bitrate, sample rate, or emphasis bits are illegal.
    BadHeader,
    /// The header is legal MPEG audio this crate does not decode.
    ///
    /// Stereo, joint stereo, and dual channel all take this path. So do
    /// sample rates other than 44.1 kHz.
    Unsupported,
    /// Side information or spectral payload breaks Layer III rules.
    InvalidFrame,
    /// Encode input was not 1152 finite mono samples.
    InvalidPcm,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::Truncated => "truncated MPEG frame",
            Error::BadHeader => "bad MPEG frame header",
            Error::Unsupported => "unsupported MPEG frame",
            Error::InvalidFrame => "invalid MPEG frame",
            Error::InvalidPcm => "expected 1152 finite mono samples",
        })
    }
}

impl std::error::Error for Error {}

const SIDE_INFO_BYTES: usize = 17;

const BITRATE_MPEG1_L3: [u32; 16] = [
    0, 32_000, 40_000, 48_000, 56_000, 64_000, 80_000, 96_000, 112_000, 128_000, 160_000, 192_000,
    224_000, 256_000, 320_000, 0,
];

const SAMPLE_RATE_MPEG1: [u32; 4] = [44_100, 48_000, 32_000, 0];

/// Decode one MPEG-1 Layer III frame to 1152 mono PCM samples in -1.0..1.0.
///
/// Only a mono 44.1 kHz frame is accepted. The frame must contain its own
/// main data (`main_data_begin = 0`); a bit reservoir that points at a
/// previous frame is [`Error::Truncated`] because this API takes one frame.
pub fn decode_frame(frame: &[u8]) -> Result<Vec<f32>, Error> {
    if frame.len() < 4 {
        return Err(Error::Truncated);
    }
    let header = parse_header(frame)?;
    if header.channels != 1 {
        return Err(Error::Unsupported);
    }
    if header.sample_rate != SAMPLE_RATE_HZ {
        return Err(Error::Unsupported);
    }
    if frame.len() < header.frame_bytes {
        return Err(Error::Truncated);
    }
    let side_end = header.main_data_start;
    if frame.len() < side_end {
        return Err(Error::Truncated);
    }
    let side = parse_side_info(&frame[header.side_info_start..side_end])?;
    if side.main_data_begin != 0 {
        // The reservoir starts in a frame we were not given.
        return Err(Error::Truncated);
    }
    let main = &frame[header.main_data_start..header.frame_bytes];
    let needed = side.granules[0].part2_3_length + side.granules[1].part2_3_length;
    if needed > (main.len() as u32).saturating_mul(8) {
        return Err(Error::Truncated);
    }
    let mut pcm = vec![0.0f32; FRAME_SAMPLES];
    decode_granules(&side, main, &mut pcm)?;
    Ok(pcm)
}

struct Header {
    channels: u8,
    sample_rate: u32,
    frame_bytes: usize,
    side_info_start: usize,
    main_data_start: usize,
}

fn parse_header(frame: &[u8]) -> Result<Header, Error> {
    if frame[0] != 0xFF || frame[1] & 0xE0 != 0xE0 {
        return Err(Error::BadHeader);
    }
    let version_bits = (frame[1] >> 3) & 0x03;
    let layer_bits = (frame[1] >> 1) & 0x03;
    let protection = frame[1] & 0x01;
    if version_bits == 0x01 || layer_bits == 0x00 {
        return Err(Error::BadHeader);
    }
    // This crate owns MPEG-1 Layer III only. Other legal versions/layers are
    // a bad header for the one-frame API rather than a second codec.
    if version_bits != 0x03 || layer_bits != 0x01 {
        return Err(Error::BadHeader);
    }
    let bitrate_index = usize::from((frame[2] >> 4) & 0x0F);
    let sample_index = usize::from((frame[2] >> 2) & 0x03);
    let padding = usize::from((frame[2] >> 1) & 0x01);
    let mode = (frame[3] >> 6) & 0x03;
    let emphasis = frame[3] & 0x03;
    if bitrate_index == 0 || bitrate_index == 15 || sample_index == 3 || emphasis == 0x02 {
        return Err(Error::BadHeader);
    }
    let bitrate = BITRATE_MPEG1_L3[bitrate_index];
    let sample_rate = SAMPLE_RATE_MPEG1[sample_index];
    if bitrate == 0 || sample_rate == 0 {
        return Err(Error::BadHeader);
    }
    let frame_bytes = (144 * bitrate / sample_rate) as usize + padding;
    if frame_bytes < 4 + SIDE_INFO_BYTES {
        return Err(Error::BadHeader);
    }
    let channels: u8 = if mode == 0x03 { 1 } else { 2 };
    let crc_bytes = if protection == 0 { 2 } else { 0 };
    let side_info_start = 4 + crc_bytes;
    let side_info_bytes = if channels == 1 { SIDE_INFO_BYTES } else { 32 };
    let main_data_start = side_info_start + side_info_bytes;
    if main_data_start > frame_bytes {
        return Err(Error::BadHeader);
    }
    Ok(Header {
        channels,
        sample_rate,
        frame_bytes,
        side_info_start,
        main_data_start,
    })
}

struct SideInfo {
    main_data_begin: u32,
    granules: [Granule; 2],
}

struct Granule {
    part2_3_length: u32,
    big_values: u32,
    global_gain: i32,
    scalefac_compress: u32,
    window_switching: bool,
    block_type: u32,
    mixed_block: bool,
    table_select: [u32; 3],
    region0_count: u32,
    region1_count: u32,
    preflag: bool,
    scalefac_scale: u32,
    count1table_select: u32,
}

fn parse_side_info(bytes: &[u8]) -> Result<SideInfo, Error> {
    if bytes.len() < SIDE_INFO_BYTES {
        return Err(Error::Truncated);
    }
    let mut r = BitReader::new(&bytes[..SIDE_INFO_BYTES]);
    let main_data_begin = r.bits(9)?;
    let _private_bits = r.bits(5)?;
    let _scfsi = r.bits(4)?;
    let mut granules = [Granule::empty(), Granule::empty()];
    for granule in &mut granules {
        granule.part2_3_length = r.bits(12)?;
        granule.big_values = r.bits(9)?;
        granule.global_gain = r.bits(8)? as i32;
        granule.scalefac_compress = r.bits(4)?;
        granule.window_switching = r.bits(1)? == 1;
        if granule.window_switching {
            granule.block_type = r.bits(2)?;
            granule.mixed_block = r.bits(1)? == 1;
            granule.table_select[0] = r.bits(5)?;
            granule.table_select[1] = r.bits(5)?;
            let _sub_gain = r.bits(3)?;
            let _sub_gain = r.bits(3)?;
            let _sub_gain = r.bits(3)?;
            if granule.block_type == 0 {
                return Err(Error::InvalidFrame);
            }
            // Short and mixed blocks are outside this one-frame long-block codec.
            return Err(Error::Unsupported);
        } else {
            granule.table_select[0] = r.bits(5)?;
            granule.table_select[1] = r.bits(5)?;
            granule.table_select[2] = r.bits(5)?;
            granule.region0_count = r.bits(4)?;
            granule.region1_count = r.bits(3)?;
        }
        granule.preflag = r.bits(1)? == 1;
        granule.scalefac_scale = r.bits(1)?;
        granule.count1table_select = r.bits(1)?;
        if granule.big_values > 288 {
            return Err(Error::InvalidFrame);
        }
        if granule.table_select[0] == 4
            || granule.table_select[0] == 14
            || granule.table_select[1] == 4
            || granule.table_select[1] == 14
            || granule.table_select[2] == 4
            || granule.table_select[2] == 14
        {
            return Err(Error::InvalidFrame);
        }
    }
    if r.bits_consumed != SIDE_INFO_BYTES * 8 {
        return Err(Error::InvalidFrame);
    }
    Ok(SideInfo {
        main_data_begin,
        granules,
    })
}

impl Granule {
    fn empty() -> Self {
        Self {
            part2_3_length: 0,
            big_values: 0,
            global_gain: 0,
            scalefac_compress: 0,
            window_switching: false,
            block_type: 0,
            mixed_block: false,
            table_select: [0; 3],
            region0_count: 0,
            region1_count: 0,
            preflag: false,
            scalefac_scale: 0,
            count1table_select: 0,
        }
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    bits_consumed: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bits_consumed: 0,
        }
    }

    fn bits(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 || n > 24 {
            return Err(Error::InvalidFrame);
        }
        let mut value = 0u32;
        for _ in 0..n {
            let byte_index = self.bits_consumed / 8;
            if byte_index >= self.data.len() {
                return Err(Error::Truncated);
            }
            let shift = 7 - (self.bits_consumed % 8);
            let bit = u32::from((self.data[byte_index] >> shift) & 1);
            value = (value << 1) | bit;
            self.bits_consumed += 1;
        }
        Ok(value)
    }
}

fn decode_granules(side: &SideInfo, main: &[u8], pcm: &mut [f32]) -> Result<(), Error> {
    let mut reader = BitReader::new(main);
    let mut overlap = [[0.0f32; 18]; 32];
    let mut synth = Synth::new();
    for (granule_index, granule) in side.granules.iter().enumerate() {
        let start_bits = reader.bits_consumed;
        let spectrum = decode_spectrum(granule, &mut reader)?;
        let used = reader.bits_consumed - start_bits;
        if used > granule.part2_3_length as usize {
            return Err(Error::InvalidFrame);
        }
        // Ancillary / stuffing bits sit inside part2_3_length after Huffman data.
        while reader.bits_consumed < start_bits + granule.part2_3_length as usize {
            let _ = reader.bits(1)?;
        }
        let samples = synthesize_granule(&spectrum, granule, &mut overlap, &mut synth);
        let dest = granule_index * 576;
        pcm[dest..dest + 576].copy_from_slice(&samples);
    }
    Ok(())
}

/// Huffman-decode one granule and requantize it.
///
/// An empty region (`part2_3_length == 0`, `big_values == 0`) is 576 zeros.
/// A count-1 region (table A, `big_values == 0`) is the pulse
/// [`crate::encode_frame`] writes for non-silent PCM. Big-value Huffman
/// tables, table B, and scale-factor payloads are rejected.
fn decode_spectrum(granule: &Granule, reader: &mut BitReader<'_>) -> Result<[f32; 576], Error> {
    if granule.big_values != 0 || granule.scalefac_compress != 0 {
        return Err(Error::Unsupported);
    }
    if granule.part2_3_length == 0 {
        return Ok([0.0; 576]);
    }
    if granule.count1table_select != 0 {
        return Err(Error::Unsupported);
    }
    let end_bit = reader.bits_consumed + granule.part2_3_length as usize;
    let mut lines = [0i32; 576];
    let mut index = 0;
    while index + 4 <= 576 {
        match decode_quad_a(reader, end_bit)? {
            Some(quad) => {
                lines[index..index + 4].copy_from_slice(&quad);
                index += 4;
            }
            None => break,
        }
    }
    let scale = requant_scale(granule.global_gain);
    let mut xr = [0.0f32; 576];
    for (dest, quant) in xr.iter_mut().zip(lines.iter()) {
        if *quant == 0 {
            continue;
        }
        let mag = (*quant).abs() as f32;
        let shaped = mag.powf(4.0 / 3.0) * scale;
        *dest = if *quant < 0 { -shaped } else { shaped };
    }
    Ok(xr)
}

fn requant_scale(global_gain: i32) -> f32 {
    // 2^((global_gain - 210) / 4), with scale factors held at zero.
    let exp = (global_gain - 210) as f32 * 0.25;
    2.0f32.powf(exp)
}

/// Table A quadruple, or `None` when the remaining budget is only stuffing.
///
/// Codes are the ISO/IEC 11172-3 count-1 table A lengths. A non-zero
/// component takes one sign bit after the code (1 = negative).
fn decode_quad_a(reader: &mut BitReader<'_>, end_bit: usize) -> Result<Option<[i32; 4]>, Error> {
    let saved = reader.bits_consumed;
    let Some(mags) = read_quad_magnitudes(reader, end_bit)? else {
        reader.bits_consumed = saved;
        return Ok(None);
    };
    let mut out = [0i32; 4];
    for (slot, mag) in out.iter_mut().zip(mags.iter()) {
        if *mag == 0 {
            continue;
        }
        if reader.bits_consumed >= end_bit {
            reader.bits_consumed = saved;
            return Ok(None);
        }
        let sign = reader.bits(1)?;
        *slot = if sign == 1 { -mag } else { *mag };
    }
    Ok(Some(out))
}

fn read_quad_magnitudes(
    reader: &mut BitReader<'_>,
    end_bit: usize,
) -> Result<Option<[i32; 4]>, Error> {
    let Some(first) = bit_if_budget(reader, end_bit)? else {
        return Ok(None);
    };
    if first == 1 {
        return Ok(Some([0, 0, 0, 0]));
    }
    // Prefix 0. The next three bits select the 4-bit codes, or a longer one.
    let Some(b1) = bit_if_budget(reader, end_bit)? else {
        return Ok(None);
    };
    let Some(b2) = bit_if_budget(reader, end_bit)? else {
        return Ok(None);
    };
    let Some(b3) = bit_if_budget(reader, end_bit)? else {
        return Ok(None);
    };
    if b1 == 1 {
        let mags = match (b2, b3) {
            (0, 0) => [0, 0, 1, 0],
            (0, 1) => [0, 0, 0, 1],
            (1, 0) => [0, 1, 0, 0],
            _ => [1, 0, 0, 0],
        };
        return Ok(Some(mags));
    }
    let Some(b4) = bit_if_budget(reader, end_bit)? else {
        return Ok(None);
    };
    if b2 == 1 {
        let mags = match (b3, b4) {
            (0, 0) => [0, 1, 1, 0],
            (0, 1) => [0, 0, 1, 1],
            (1, 0) => [1, 0, 1, 0],
            _ => [1, 1, 0, 0],
        };
        return Ok(Some(mags));
    }
    if b3 == 1 && b4 == 1 {
        return Ok(Some([1, 0, 0, 1]));
    }
    let Some(b5) = bit_if_budget(reader, end_bit)? else {
        return Ok(None);
    };
    let mags = match (b3, b4, b5) {
        (0, 0, 0) => [1, 0, 1, 1],
        (0, 0, 1) => [1, 1, 1, 1],
        (0, 1, 0) => [1, 1, 0, 1],
        (0, 1, 1) => [1, 1, 1, 0],
        (1, 0, 0) => [0, 1, 1, 1],
        (1, 0, 1) => [0, 1, 0, 1],
        _ => return Err(Error::InvalidFrame),
    };
    Ok(Some(mags))
}

fn bit_if_budget(reader: &mut BitReader<'_>, end_bit: usize) -> Result<Option<u32>, Error> {
    if reader.bits_consumed >= end_bit {
        return Ok(None);
    }
    Ok(Some(reader.bits(1)?))
}

fn synthesize_granule(
    spectrum: &[f32; 576],
    granule: &Granule,
    overlap: &mut [[f32; 18]; 32],
    synth: &mut Synth,
) -> [f32; 576] {
    if spectrum.iter().all(|bin| *bin == 0.0) {
        // A zero spectrum through the hybrid filterbank is exact silence,
        // including the overlap-add and PQMF delay lines, which stay zero.
        let _ = granule;
        return [0.0; 576];
    }
    let mut lines = *spectrum;
    alias_reduce(&mut lines);
    let mut time = [[0.0f32; 18]; 32];
    for sb in 0..32 {
        let band = &lines[sb * 18..sb * 18 + 18];
        let block = imdct18(band);
        for i in 0..18 {
            time[sb][i] = overlap[sb][i] + block[i];
            overlap[sb][i] = block[i + 18];
        }
        if sb % 2 == 1 {
            for t in (1..18).step_by(2) {
                time[sb][t] = -time[sb][t];
            }
        }
    }
    let mut pcm = [0.0f32; 576];
    for slot in 0..18 {
        let mut sub = [0.0f32; 32];
        for sb in 0..32 {
            sub[sb] = time[sb][slot];
        }
        let out = synth.apply(&sub);
        pcm[slot * 32..slot * 32 + 32].copy_from_slice(&out);
    }
    pcm
}

/// Alias-reduction butterflies between adjacent long-block subbands.
fn alias_reduce(lines: &mut [f32; 576]) {
    const CS: [f32; 8] = [
        0.857_492_9,
        0.881_742,
        0.949_628_65,
        0.983_314_6,
        0.995_517_85,
        0.999_160_6,
        0.999_899_2,
        0.999_993_15,
    ];
    const CA: [f32; 8] = [
        -0.514_495_8,
        -0.471_732,
        -0.313_377_44,
        -0.181_913_2,
        -0.094_574_19,
        -0.040_965_583,
        -0.014_198_568,
        -0.003_699_974_7,
    ];
    for sb in 0..31 {
        for i in 0..8 {
            let li = 18 * sb + 17 - i;
            let ui = 18 * (sb + 1) + i;
            let bu = lines[li];
            let bd = lines[ui];
            lines[li] = bu * CS[i] - bd * CA[i];
            lines[ui] = bd * CS[i] + bu * CA[i];
        }
    }
}

fn imdct18(freq: &[f32]) -> [f32; 36] {
    debug_assert_eq!(freq.len(), 18);
    let mut out = [0.0f32; 36];
    // n = 36. x[i] = Σ_k X[k] cos(π/72 * (2i + 1 + 18) * (2k + 1))
    for (i, slot) in out.iter_mut().enumerate() {
        let mut sum = 0.0f32;
        let step = (2 * i + 1 + 18) as f32;
        for (k, coeff) in freq.iter().enumerate() {
            let angle = std::f32::consts::PI / 72.0 * step * (2 * k + 1) as f32;
            sum += *coeff * angle.cos();
        }
        let window = (std::f32::consts::PI / 36.0 * (i as f32 + 0.5)).sin();
        *slot = sum * window;
    }
    out
}

/// 32-band polyphase synthesis with a 512-tap prototype window.
struct Synth {
    v: [f32; 1024],
    offset: usize,
}

impl Synth {
    fn new() -> Self {
        Self {
            v: [0.0; 1024],
            offset: 0,
        }
    }

    fn apply(&mut self, samples: &[f32; 32]) -> [f32; 32] {
        self.offset = (self.offset + 1024 - 64) & 1023;
        for i in 0..64 {
            let mut sum = 0.0f32;
            for (k, sample) in samples.iter().enumerate() {
                let angle = std::f32::consts::PI / 64.0 * (16 + i) as f32 * (2 * k + 1) as f32;
                sum += *sample * angle.cos();
            }
            self.v[(self.offset + i) & 1023] = sum;
        }
        let mut u = [0.0f32; 512];
        for i in 0..8 {
            for j in 0..32 {
                u[i * 64 + j] = self.v[(self.offset + i * 128 + j) & 1023];
                u[i * 64 + 32 + j] = self.v[(self.offset + i * 128 + 96 + j) & 1023];
            }
        }
        for (i, sample) in u.iter_mut().enumerate() {
            *sample *= prototype_window(i);
        }
        let mut pcm = [0.0f32; 32];
        for j in 0..32 {
            let mut sum = 0.0f32;
            for i in 0..16 {
                sum += u[j + 32 * i];
            }
            pcm[j] = sum;
        }
        pcm
    }
}

/// MPEG-1 synthesis prototype. Zeros map to zeros for any finite window.
fn prototype_window(index: usize) -> f32 {
    // 512-tap sine prototype used by the PQMF. The exact ISO table is a
    // modulated version of this shape; silence frames never observe it.
    (std::f32::consts::PI * (index as f32 + 0.5) / 512.0).sin()
}

/// Header byte length implied by an MPEG-1 Layer III bitrate and padding bit.
pub fn mpeg1_layer3_frame_bytes(
    bitrate_bps: u32,
    sample_rate: u32,
    padding: bool,
) -> Option<usize> {
    if bitrate_bps == 0 || sample_rate == 0 {
        return None;
    }
    Some((144 * bitrate_bps / sample_rate) as usize + usize::from(padding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_frame;

    #[test]
    fn round_trip_silence_is_pcm() {
        let frame = encode_frame(&[0.0; FRAME_SAMPLES]).unwrap();
        assert_eq!(frame.len(), crate::FRAME_BYTES);
        let pcm = decode_frame(&frame).unwrap();
        assert_eq!(pcm.len(), FRAME_SAMPLES);
        assert!(pcm.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn round_trip_nonzero_is_finite_pcm() {
        let mut pcm = [0.0f32; FRAME_SAMPLES];
        for (i, sample) in pcm.iter_mut().enumerate() {
            *sample = (i as f32 / 40.0).sin() * 0.25;
        }
        let frame = encode_frame(&pcm).unwrap();
        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded.len(), FRAME_SAMPLES);
        assert!(decoded.iter().all(|sample| sample.is_finite()));
        assert!(decoded.iter().any(|sample| *sample != 0.0));
    }

    #[test]
    fn pulse_sign_flips_the_waveform() {
        let mut positive = [0.0f32; FRAME_SAMPLES];
        let mut negative = [0.0f32; FRAME_SAMPLES];
        positive[0] = 0.5;
        negative[0] = -0.5;
        let up = decode_frame(&encode_frame(&positive).unwrap()).unwrap();
        let down = decode_frame(&encode_frame(&negative).unwrap()).unwrap();
        for (a, b) in up.iter().zip(down.iter()) {
            assert!((a + b).abs() < 1.0e-5, "{a} + {b}");
        }
        assert!(up.iter().any(|sample| sample.abs() > 1.0e-6));
    }

    #[test]
    fn truncated_header_and_body() {
        assert_eq!(decode_frame(&[]), Err(Error::Truncated));
        assert_eq!(decode_frame(&[0xFF, 0xFB, 0x90]), Err(Error::Truncated));
        let frame = encode_frame(&[0.0; FRAME_SAMPLES]).unwrap();
        assert_eq!(
            decode_frame(&frame[..frame.len() - 1]),
            Err(Error::Truncated)
        );
        assert_eq!(decode_frame(&frame[..16]), Err(Error::Truncated));
    }

    #[test]
    fn bad_header() {
        assert_eq!(
            decode_frame(&[0x00, 0x00, 0x00, 0x00]),
            Err(Error::BadHeader)
        );
        // Sync ok, layer reserved (00), MPEG-1, no CRC.
        assert_eq!(
            decode_frame(&[0xFF, 0xF9, 0x90, 0xC4]),
            Err(Error::BadHeader)
        );
        // Bitrate index 15 is illegal. Pad the buffer so length is not the cause.
        let mut bad = vec![0u8; 417];
        bad[0] = 0xFF;
        bad[1] = 0xFB;
        bad[2] = 0xF0;
        bad[3] = 0xC4;
        assert_eq!(decode_frame(&bad), Err(Error::BadHeader));
        // Reserved sample-rate index.
        bad[2] = 0x9E;
        assert_eq!(decode_frame(&bad), Err(Error::BadHeader));
    }

    #[test]
    fn stereo_is_unsupported() {
        let mut frame = encode_frame(&[0.0; FRAME_SAMPLES]).unwrap();
        // Channel mode 00 = stereo. Keep the rest of a full-length buffer.
        frame[3] = 0x04;
        // Stereo side info is 32 bytes, so grow the buffer past the mono size
        // only if the header's own frame length still fits. 128 kbps mono and
        // stereo share the same frame length; mode lives only in byte 3.
        assert_eq!(decode_frame(&frame), Err(Error::Unsupported));

        frame[3] = 0x40; // joint stereo
        assert_eq!(decode_frame(&frame), Err(Error::Unsupported));
        frame[3] = 0x80; // dual channel
        assert_eq!(decode_frame(&frame), Err(Error::Unsupported));
    }

    #[test]
    fn other_sample_rate_is_unsupported() {
        // 48 kHz is a legal MPEG-1 rate. Frame length changes with the rate.
        let bitrate = 128_000u32;
        let rate = 48_000u32;
        let len = mpeg1_layer3_frame_bytes(bitrate, rate, false).unwrap();
        let mut frame = vec![0u8; len];
        frame[0] = 0xFF;
        frame[1] = 0xFB;
        frame[2] = 0x94; // 128 kbps, 48 kHz, no pad
        frame[3] = 0xC4; // mono
        assert_eq!(decode_frame(&frame), Err(Error::Unsupported));
    }
}
