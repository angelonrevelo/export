//! ADTS header plus one mono AAC-LC raw data block.

use crate::bits::{BitReader, BitWriter};
use crate::huff;
use crate::mdct;
use crate::FRAME_SAMPLES;

/// Long-window scale-factor band edges at 44.1 kHz (49 bands).
const SWB: [u16; 50] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 88, 96, 108, 120, 132, 144, 160,
    176, 196, 216, 240, 264, 292, 320, 352, 384, 416, 448, 480, 512, 544, 576, 608, 640, 672, 704,
    736, 768, 800, 832, 864, 896, 928, 1024,
];

const NUM_SWB: usize = 49;
const ZERO_HCB: u32 = 0;
const ESC_HCB: u32 = 11;
const ID_SCE: u32 = 0;
const ID_FIL: u32 = 6;
const ID_END: u32 = 7;
const EXT_SBR_DATA: u32 = 13;
const EXT_SBR_DATA_CRC: u32 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The buffer ended before a full header or a full declared frame.
    Truncated,
    /// Sync, layer, or frame length is not a valid ADTS header.
    BadHeader,
    /// HE-AAC, spectral band replication, or parametric stereo.
    Unsupported,
    /// The frame is not one mono AAC-LC long-window block at 44.1 kHz.
    Invalid,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::Truncated => "truncated AAC frame",
            Error::BadHeader => "bad ADTS header",
            Error::Unsupported => "unsupported AAC feature: HE-AAC, SBR, or PS",
            Error::Invalid => "invalid AAC frame",
        })
    }
}

impl std::error::Error for Error {}

/// Encode 1024 mono PCM samples in about [-1, 1] as one AAC-LC ADTS frame.
///
/// The left half of the long window is silence. This call does not keep
/// overlap for a later frame.
pub fn encode_frame(samples: &[f32]) -> Result<Vec<u8>, Error> {
    encode_frame_following(&[0.0; FRAME_SAMPLES], samples)
}

/// Encode `samples` as one AAC-LC ADTS frame whose long window starts with
/// `previous`.
///
/// `previous` is the prior 1024 PCM samples, not a decoded overlap tail.
/// The library does not store it. [`encode_second_frame`] passes
/// [`crate::one_frame_pcm`] here. [`encode_third_frame`] passes
/// [`crate::second_frame_pcm`]. [`encode_fourth_frame`] passes
/// [`crate::third_frame_pcm`].
pub fn encode_frame_following(previous: &[f32], samples: &[f32]) -> Result<Vec<u8>, Error> {
    if previous.len() != FRAME_SAMPLES || samples.len() != FRAME_SAMPLES {
        return Err(Error::Invalid);
    }
    if previous
        .iter()
        .chain(samples.iter())
        .any(|s| !s.is_finite())
    {
        return Err(Error::Invalid);
    }

    let mut block = [0.0f64; 2048];
    for i in 0..FRAME_SAMPLES {
        block[i] = f64::from(previous[i]);
        block[FRAME_SAMPLES + i] = f64::from(samples[i]);
    }
    let spec = mdct::analysis(&block);
    let (global_gain, quant) = quantize(&spec);
    let payload = write_raw_data_block(global_gain, &quant)?;
    wrap_adts(&payload)
}

/// The recorded second frame: left half [`crate::one_frame_pcm`], right half
/// the next 1024 samples of that same half-scale 440 Hz sine.
pub fn encode_second_frame() -> Result<Vec<u8>, Error> {
    encode_frame_following(&crate::one_frame_pcm(), &crate::second_frame_pcm())
}

/// The recorded third frame: left half [`crate::second_frame_pcm`], right half
/// the next 1024 samples of that same half-scale 440 Hz sine.
pub fn encode_third_frame() -> Result<Vec<u8>, Error> {
    encode_frame_following(&crate::second_frame_pcm(), &crate::third_frame_pcm())
}

/// The recorded fourth frame: left half [`crate::third_frame_pcm`], right half
/// the next 1024 samples of that same half-scale 440 Hz sine.
pub fn encode_fourth_frame() -> Result<Vec<u8>, Error> {
    encode_frame_following(&crate::third_frame_pcm(), &crate::fourth_frame_pcm())
}

/// Decode one AAC-LC ADTS frame to 1024 mono PCM samples.
///
/// Overlap history is zero, so a lone frame does not reconstruct its input.
pub fn decode_frame(bytes: &[u8]) -> Result<[f32; FRAME_SAMPLES], Error> {
    let spec = spectrum(bytes)?;
    Ok(pcm_f32(&mdct::synthesis_one_frame(&spec)))
}

/// Decode `second` overlap-added against the tail of `first`.
///
/// The samples are what a decoder emits for the second frame. With the
/// recorded pair, that is the first frame's sine, one frame late. It is not
/// [`crate::second_frame_pcm`].
pub fn decode_second_frame(first: &[u8], second: &[u8]) -> Result<[f32; FRAME_SAMPLES], Error> {
    let spec0 = spectrum(first)?;
    let mut overlap = [0.0; FRAME_SAMPLES];
    let _ = mdct::synthesis_overlap(&spec0, &mut overlap);
    let spec1 = spectrum(second)?;
    Ok(pcm_f32(&mdct::synthesis_overlap(&spec1, &mut overlap)))
}

/// Decode `third` overlap-added against the tail left by `second`.
///
/// `first` is decoded only to prime that tail. With the recorded triple, the
/// samples are [`crate::second_frame_pcm`], still one frame late. They are not
/// [`crate::third_frame_pcm`].
pub fn decode_third_frame(
    first: &[u8],
    second: &[u8],
    third: &[u8],
) -> Result<[f32; FRAME_SAMPLES], Error> {
    let spec0 = spectrum(first)?;
    let mut overlap = [0.0; FRAME_SAMPLES];
    let _ = mdct::synthesis_overlap(&spec0, &mut overlap);
    let spec1 = spectrum(second)?;
    let _ = mdct::synthesis_overlap(&spec1, &mut overlap);
    let spec2 = spectrum(third)?;
    Ok(pcm_f32(&mdct::synthesis_overlap(&spec2, &mut overlap)))
}

/// Decode `fourth` overlap-added against the tail left by `third`.
///
/// Earlier frames are decoded only to prime that tail. With the recorded
/// series, the samples are [`crate::third_frame_pcm`], still one frame late.
/// They are not [`crate::fourth_frame_pcm`].
pub fn decode_fourth_frame(
    first: &[u8],
    second: &[u8],
    third: &[u8],
    fourth: &[u8],
) -> Result<[f32; FRAME_SAMPLES], Error> {
    let spec0 = spectrum(first)?;
    let mut overlap = [0.0; FRAME_SAMPLES];
    let _ = mdct::synthesis_overlap(&spec0, &mut overlap);
    let spec1 = spectrum(second)?;
    let _ = mdct::synthesis_overlap(&spec1, &mut overlap);
    let spec2 = spectrum(third)?;
    let _ = mdct::synthesis_overlap(&spec2, &mut overlap);
    let spec3 = spectrum(fourth)?;
    Ok(pcm_f32(&mdct::synthesis_overlap(&spec3, &mut overlap)))
}

fn spectrum(bytes: &[u8]) -> Result<[f64; FRAME_SAMPLES], Error> {
    let adts = parse_adts(bytes)?;
    read_raw_data_block(adts.payload)
}

fn pcm_f32(pcm: &[f64; FRAME_SAMPLES]) -> [f32; FRAME_SAMPLES] {
    let mut out = [0.0f32; FRAME_SAMPLES];
    for (i, sample) in pcm.iter().enumerate() {
        out[i] = *sample as f32;
    }
    out
}

fn wrap_adts(payload: &[u8]) -> Result<Vec<u8>, Error> {
    let frame_len = 7 + payload.len();
    if frame_len > 0x1FFF {
        return Err(Error::Invalid);
    }
    let mut out = Vec::with_capacity(frame_len);
    out.extend_from_slice(&adts_header(frame_len)?);
    out.extend_from_slice(payload);
    Ok(out)
}

fn adts_header(frame_len: usize) -> Result<[u8; 7], Error> {
    if !(7..=0x1FFF).contains(&frame_len) {
        return Err(Error::BadHeader);
    }
    let mut h = [0u8; 7];
    // sync 12 bits, MPEG-4 id, layer 00, protection absent.
    h[0] = 0xFF;
    h[1] = 0xF1;
    // profile AAC-LC (1), 44100 Hz (index 4), private 0, channel msb 0.
    h[2] = 0x50;
    // channel low bits 01, original/home/copyright 0, frame length top 2 bits.
    h[3] = 0b0100_0000 | ((frame_len >> 11) as u8 & 0x03);
    h[4] = (frame_len >> 3) as u8;
    // frame length low 3 bits, buffer fullness 0x7FF (VBR) top 5 bits.
    h[5] = ((frame_len as u8 & 0x07) << 5) | 0x1F;
    // buffer fullness low 6 bits, one raw data block.
    h[6] = 0xFC;
    Ok(h)
}

struct Adts<'a> {
    payload: &'a [u8],
}

fn parse_adts(data: &[u8]) -> Result<Adts<'_>, Error> {
    if data.len() < 7 {
        return Err(Error::Truncated);
    }
    if data[0] != 0xFF || data[1] & 0xF0 != 0xF0 {
        return Err(Error::BadHeader);
    }
    let layer = (data[1] >> 1) & 0x03;
    if layer != 0 {
        return Err(Error::BadHeader);
    }
    let protection_absent = data[1] & 0x01;
    let profile = (data[2] >> 6) & 0x03;
    let sample_rate_index = (data[2] >> 2) & 0x0F;
    let channel = ((data[2] & 0x01) << 2) | (data[3] >> 6);
    let frame_len = (usize::from(data[3] & 0x03) << 11)
        | (usize::from(data[4]) << 3)
        | (usize::from(data[5]) >> 5);
    let blocks = data[6] & 0x03;
    if sample_rate_index == 0x0F || frame_len < 7 {
        return Err(Error::BadHeader);
    }
    let header_len = if protection_absent == 1 { 7 } else { 9 };
    if frame_len < header_len {
        return Err(Error::BadHeader);
    }
    if data.len() < frame_len {
        return Err(Error::Truncated);
    }
    // Profile is audioObjectType - 1. HE-AAC (AOT 5 / 29) does not fit in
    // these two bits; SBR and PS are detected in the fill element. Other
    // profiles are not this frame.
    if profile != 1 || sample_rate_index != 4 || channel != 1 || blocks != 0 {
        return Err(Error::Invalid);
    }
    Ok(Adts {
        payload: &data[header_len..frame_len],
    })
}

fn write_raw_data_block(global_gain: u8, quant: &[i32; FRAME_SAMPLES]) -> Result<Vec<u8>, Error> {
    let sections = sections(quant);
    let max_sfb = sections.iter().map(|s| s.sfb0 + s.len).max().unwrap_or(0);
    let mut w = BitWriter::new();
    w.put(ID_SCE, 3);
    w.put(0, 4); // element_instance_tag
    w.put(u32::from(global_gain), 8);
    write_ics_info(&mut w, max_sfb as u32);
    for section in &sections {
        w.put(section.cb, 4);
        write_section_length(&mut w, section.len);
    }
    for section in &sections {
        if section.cb == ZERO_HCB {
            continue;
        }
        for _ in 0..section.len {
            huff::write_sf_delta_zero(&mut w)?;
        }
    }
    w.put(0, 1); // pulse_data_present
    w.put(0, 1); // tns_data_present
    w.put(0, 1); // gain_control_data_present
    for section in &sections {
        if section.cb == ZERO_HCB {
            continue;
        }
        for sfb in section.sfb0..section.sfb0 + section.len {
            let start = usize::from(SWB[sfb]);
            let end = usize::from(SWB[sfb + 1]);
            let mut i = start;
            while i < end {
                huff::write_cb11_pair(&mut w, quant[i], quant[i + 1])?;
                i += 2;
            }
        }
    }
    w.put(ID_END, 3);
    Ok(w.finish())
}

fn write_ics_info(w: &mut BitWriter, max_sfb: u32) {
    w.put(0, 1); // ics_reserved_bit
    w.put(0, 2); // ONLY_LONG_SEQUENCE
    w.put(0, 1); // sine window
    w.put(max_sfb, 6);
    w.put(0, 1); // predictor_data_present
}

fn write_section_length(w: &mut BitWriter, len: usize) {
    const ESC: usize = 31;
    let mut rest = len;
    while rest >= ESC {
        w.put(ESC as u32, 5);
        rest -= ESC;
    }
    w.put(rest as u32, 5);
}

struct Section {
    cb: u32,
    sfb0: usize,
    len: usize,
}

fn sections(quant: &[i32; FRAME_SAMPLES]) -> Vec<Section> {
    let mut max_sfb = 0usize;
    for sfb in 0..NUM_SWB {
        let start = usize::from(SWB[sfb]);
        let end = usize::from(SWB[sfb + 1]);
        if quant[start..end].iter().any(|q| *q != 0) {
            max_sfb = sfb + 1;
        }
    }
    let mut out = Vec::new();
    let mut sfb = 0;
    while sfb < max_sfb {
        let cb = band_cb(quant, sfb);
        let mut len = 1;
        while sfb + len < max_sfb && band_cb(quant, sfb + len) == cb {
            len += 1;
        }
        out.push(Section { cb, sfb0: sfb, len });
        sfb += len;
    }
    out
}

fn band_cb(quant: &[i32; FRAME_SAMPLES], sfb: usize) -> u32 {
    let start = usize::from(SWB[sfb]);
    let end = usize::from(SWB[sfb + 1]);
    if quant[start..end].iter().any(|q| *q != 0) {
        ESC_HCB
    } else {
        ZERO_HCB
    }
}

fn read_raw_data_block(payload: &[u8]) -> Result<[f64; FRAME_SAMPLES], Error> {
    let mut r = BitReader::new(payload);
    let mut spec = [0.0f64; FRAME_SAMPLES];
    let mut saw_sce = false;
    loop {
        let id = r.read(3)?;
        match id {
            ID_SCE => {
                if saw_sce {
                    return Err(Error::Invalid);
                }
                spec = read_sce(&mut r)?;
                saw_sce = true;
            }
            ID_FIL => read_fill(&mut r)?,
            ID_END => {
                r.byte_align()?;
                break;
            }
            _ => return Err(Error::Invalid),
        }
    }
    if !saw_sce {
        return Err(Error::Invalid);
    }
    Ok(spec)
}

fn read_sce(r: &mut BitReader<'_>) -> Result<[f64; FRAME_SAMPLES], Error> {
    let _tag = r.read(4)?;
    let global_gain = r.read(8)? as i32;
    let reserved = r.read(1)?;
    let window_sequence = r.read(2)?;
    let window_shape = r.read(1)?;
    if reserved != 0 || window_sequence != 0 || window_shape != 0 {
        return Err(Error::Invalid);
    }
    let max_sfb = r.read(6)? as usize;
    let predictor = r.read(1)?;
    if predictor != 0 || max_sfb > NUM_SWB {
        return Err(Error::Invalid);
    }
    let mut cb = [ZERO_HCB as u8; NUM_SWB];
    let mut sfb = 0usize;
    while sfb < max_sfb {
        let sect_cb = r.read(4)?;
        if sect_cb != ZERO_HCB && sect_cb != ESC_HCB {
            return Err(Error::Invalid);
        }
        let mut len = 0usize;
        loop {
            let incr = r.read(5)? as usize;
            len += incr;
            if incr != 31 {
                break;
            }
            if len > max_sfb {
                return Err(Error::Invalid);
            }
        }
        if len == 0 || sfb + len > max_sfb {
            return Err(Error::Invalid);
        }
        for band in &mut cb[sfb..sfb + len] {
            *band = sect_cb as u8;
        }
        sfb += len;
    }
    let scale = [global_gain; NUM_SWB];
    for band in &cb[..max_sfb] {
        if *band != ZERO_HCB as u8 {
            // Encoder writes delta 0, so every band stays at global_gain.
            huff::read_sf_delta_zero(r)?;
        }
    }
    if r.read(1)? != 0 || r.read(1)? != 0 || r.read(1)? != 0 {
        // pulse, TNS, or gain control — not used by the one frame we write
        return Err(Error::Invalid);
    }
    let book = huff::Cb11::new()?;
    let mut quant = [0i32; FRAME_SAMPLES];
    for sfb in 0..max_sfb {
        if cb[sfb] == ZERO_HCB as u8 {
            continue;
        }
        let start = usize::from(SWB[sfb]);
        let end = usize::from(SWB[sfb + 1]);
        if (end - start) % 2 != 0 {
            return Err(Error::Invalid);
        }
        let mut i = start;
        while i < end {
            let (a, b) = book.read_pair(r)?;
            quant[i] = a;
            quant[i + 1] = b;
            i += 2;
        }
    }
    let mut spec = [0.0f64; FRAME_SAMPLES];
    for sfb in 0..max_sfb {
        let start = usize::from(SWB[sfb]);
        let end = usize::from(SWB[sfb + 1]);
        for i in start..end {
            spec[i] = dequant(quant[i], scale[sfb]);
        }
    }
    Ok(spec)
}

fn read_fill(r: &mut BitReader<'_>) -> Result<(), Error> {
    let _tag = r.read(4)?;
    let mut count = r.read(4)?;
    if count == 15 {
        count += r.read(8)?;
    }
    if count == 0 {
        return Ok(());
    }
    // `count` is the extension payload size in bytes, starting at this bit.
    let extension_type = r.read(4)?;
    if extension_type == EXT_SBR_DATA || extension_type == EXT_SBR_DATA_CRC {
        // HE-AAC is AAC-LC plus this SBR payload. Parametric stereo is
        // carried inside the same extension (extension id PS = 2).
        return Err(Error::Unsupported);
    }
    let remaining_bits = count
        .checked_mul(8)
        .and_then(|bits| bits.checked_sub(4))
        .ok_or(Error::Invalid)?;
    for _ in 0..remaining_bits {
        let _ = r.read_bit()?;
    }
    Ok(())
}

fn quantize(spec: &[f64; FRAME_SAMPLES]) -> (u8, [i32; FRAME_SAMPLES]) {
    let peak = spec.iter().fold(0.0f64, |m, c| m.max(c.abs()));
    if peak < 1e-12 {
        return (100, [0; FRAME_SAMPLES]);
    }
    let guess = gain_for(peak, 96.0);
    let lo = guess.saturating_sub(16);
    let hi = (u16::from(guess) + 16).min(255) as u8;
    let mut best_gain = guess;
    let mut best_err = f64::MAX;
    let mut best_q = [0i32; FRAME_SAMPLES];
    let mut gain = lo;
    loop {
        let q = quantize_with(spec, gain);
        let max_abs = q.iter().fold(0i32, |m, v| m.max(v.abs()));
        if max_abs <= 4096 {
            let err = spectral_error(spec, gain, &q);
            if err < best_err {
                best_err = err;
                best_gain = gain;
                best_q = q;
            }
        }
        if gain == hi {
            break;
        }
        gain += 1;
    }
    if best_err == f64::MAX {
        (guess, quantize_with(spec, guess))
    } else {
        (best_gain, best_q)
    }
}

fn gain_for(peak: f64, q_target: f64) -> u8 {
    let scale = peak / q_target.powf(4.0 / 3.0);
    if scale <= 0.0 {
        return 100;
    }
    let gain = 100.0 + 4.0 * scale.log2();
    gain.round().clamp(0.0, 255.0) as u8
}

fn scale_of(sf: i32) -> f64 {
    2.0f64.powf((f64::from(sf) - 100.0) / 4.0)
}

fn quantize_with(spec: &[f64; FRAME_SAMPLES], gain: u8) -> [i32; FRAME_SAMPLES] {
    let scale = scale_of(i32::from(gain));
    let mut q = [0i32; FRAME_SAMPLES];
    for (i, coef) in spec.iter().enumerate() {
        if *coef == 0.0 {
            continue;
        }
        let mag = (coef.abs() / scale).powf(0.75).round();
        let mag = mag.clamp(0.0, 8191.0) as i32;
        q[i] = if *coef < 0.0 { -mag } else { mag };
    }
    q
}

fn spectral_error(spec: &[f64; FRAME_SAMPLES], gain: u8, q: &[i32; FRAME_SAMPLES]) -> f64 {
    let mut err = 0.0;
    for i in 0..FRAME_SAMPLES {
        let recon = dequant(q[i], i32::from(gain));
        let d = spec[i] - recon;
        err += d * d;
    }
    err
}

/// `sign(q) * |q|^(4/3) * 2^((sf - 100) / 4)`, with `sf` absolute.
fn dequant(q: i32, sf: i32) -> f64 {
    if q == 0 {
        return 0.0;
    }
    let mag = (f64::from(q.abs())).powf(4.0 / 3.0) * scale_of(sf);
    if q < 0 {
        -mag
    } else {
        mag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swb_table_covers_long_window() {
        assert_eq!(SWB.len(), NUM_SWB + 1);
        assert_eq!(*SWB.last().unwrap(), 1024);
        for pair in SWB.windows(2) {
            let width = pair[1] - pair[0];
            assert!(width > 0);
            assert_eq!(width % 4, 0);
        }
    }

    #[test]
    fn silence_header_and_errors() {
        let frame = encode_frame(&[0.0; FRAME_SAMPLES]).unwrap();
        assert!(frame.len() >= 7);
        assert_eq!(&frame[..2], &[0xFF, 0xF1]);
        let pcm = decode_frame(&frame).unwrap();
        assert!(pcm.iter().all(|s| *s == 0.0));

        assert_eq!(decode_frame(&frame[..4]).unwrap_err(), Error::Truncated);
        assert_eq!(
            decode_frame(&frame[..frame.len() - 1]).unwrap_err(),
            Error::Truncated
        );

        let mut bad = frame.clone();
        bad[0] = 0x00;
        assert_eq!(decode_frame(&bad).unwrap_err(), Error::BadHeader);

        let mut layer = frame.clone();
        layer[1] |= 0x02;
        assert_eq!(decode_frame(&layer).unwrap_err(), Error::BadHeader);

        assert_eq!(encode_frame(&[0.0; 8]).unwrap_err(), Error::Invalid);
    }

    #[test]
    fn one_frame_stays_inside_recorded_bound() {
        let pcm = crate::one_frame_pcm();
        let frame = encode_frame(&pcm).unwrap();
        let out = decode_frame(&frame).unwrap();
        let mut max_err = 0.0f32;
        for (a, b) in pcm.iter().zip(out.iter()) {
            max_err = max_err.max((a - b).abs());
        }
        assert!(
            max_err <= crate::RECORDED_MAX_ABS_ERROR,
            "max abs error {max_err}"
        );
        assert_eq!(frame.len(), 513);
    }

    #[test]
    fn second_frame_overlap_adds_against_the_first() {
        let sine = crate::one_frame_pcm();
        let first = encode_frame(&sine).unwrap();
        assert_eq!(first.len(), 513);
        let second = encode_second_frame().unwrap();
        let out = decode_second_frame(&first, &second).unwrap();
        let mut max_err = 0.0f32;
        let mut peak = 0.0f32;
        for (a, b) in sine.iter().zip(out.iter()) {
            max_err = max_err.max((a - b).abs());
            peak = peak.max(b.abs());
        }
        assert!(
            max_err <= crate::RECORDED_SECOND_FRAME_MAX_ABS_ERROR,
            "second frame max abs error {max_err} peak {peak}"
        );
        assert!(peak > 0.4, "second frame peak {peak} is still near silence");

        let mut truncated = second.clone();
        truncated.pop();
        assert_eq!(
            decode_second_frame(&first, &truncated).unwrap_err(),
            Error::Truncated
        );
        let sbr = fill_frame(EXT_SBR_DATA);
        assert_eq!(
            decode_second_frame(&first, &sbr).unwrap_err(),
            Error::Unsupported
        );
    }

    #[test]
    fn third_frame_is_still_one_frame_late() {
        let first_pcm = crate::one_frame_pcm();
        let second_pcm = crate::second_frame_pcm();
        let third_pcm = crate::third_frame_pcm();
        let first = encode_frame(&first_pcm).unwrap();
        let second = encode_second_frame().unwrap();
        let third = encode_third_frame().unwrap();
        assert_eq!(first.len(), 513);
        assert_eq!(second.len(), 68);
        let out = decode_third_frame(&first, &second, &third).unwrap();
        let mut late = 0.0f32;
        let mut own = 0.0f32;
        let mut peak = 0.0f32;
        for i in 0..FRAME_SAMPLES {
            late = late.max((out[i] - second_pcm[i]).abs());
            own = own.max((out[i] - third_pcm[i]).abs());
            peak = peak.max(out[i].abs());
        }
        assert!(
            late <= crate::RECORDED_THIRD_FRAME_MAX_ABS_ERROR,
            "third frame vs previous sine {late} peak {peak}"
        );
        assert!(
            own > 0.4,
            "third frame vs its own input {own} is not one frame late"
        );
        let mut truncated = third.clone();
        truncated.pop();
        assert_eq!(
            decode_third_frame(&first, &second, &truncated).unwrap_err(),
            Error::Truncated
        );
        let sbr = fill_frame(EXT_SBR_DATA);
        assert_eq!(
            decode_third_frame(&first, &second, &sbr).unwrap_err(),
            Error::Unsupported
        );
    }

    #[test]
    fn fourth_frame_is_still_one_frame_late() {
        let first_pcm = crate::one_frame_pcm();
        let third_pcm = crate::third_frame_pcm();
        let fourth_pcm = crate::fourth_frame_pcm();
        let first = encode_frame(&first_pcm).unwrap();
        let second = encode_second_frame().unwrap();
        let third = encode_third_frame().unwrap();
        let fourth = encode_fourth_frame().unwrap();
        assert_eq!(first.len(), 513);
        assert_eq!(second.len(), 68);
        assert_eq!(third.len(), 94);
        let out = decode_fourth_frame(&first, &second, &third, &fourth).unwrap();
        let mut late = 0.0f32;
        let mut own = 0.0f32;
        for i in 0..FRAME_SAMPLES {
            late = late.max((out[i] - third_pcm[i]).abs());
            own = own.max((out[i] - fourth_pcm[i]).abs());
        }
        assert!(
            late <= crate::RECORDED_FOURTH_FRAME_MAX_ABS_ERROR,
            "fourth frame vs delayed sine {late}"
        );
        assert!(
            own > 0.4,
            "fourth frame vs its own input {own} is not one frame late"
        );
        let mut truncated = fourth.clone();
        truncated.pop();
        assert_eq!(
            decode_fourth_frame(&first, &second, &third, &truncated).unwrap_err(),
            Error::Truncated
        );
        let sbr = fill_frame(EXT_SBR_DATA);
        assert_eq!(
            decode_fourth_frame(&first, &second, &third, &sbr).unwrap_err(),
            Error::Unsupported
        );
    }

    #[test]
    fn sbr_ps_and_he_aac_are_unsupported() {
        // Minimal LC ADTS shell: SCE is replaced by a fill element so the
        // SBR payload is what the decoder has to classify.
        let frame = fill_frame(EXT_SBR_DATA);
        assert_eq!(decode_frame(&frame).unwrap_err(), Error::Unsupported);
        let crc = fill_frame(EXT_SBR_DATA_CRC);
        assert_eq!(decode_frame(&crc).unwrap_err(), Error::Unsupported);
    }

    fn fill_frame(extension_type: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.put(ID_FIL, 3);
        w.put(0, 4); // tag
        w.put(1, 4); // one payload byte
        w.put(extension_type, 4);
        w.put(0, 4); // rest of that byte
        w.put(ID_END, 3);
        let payload = w.finish();
        let mut frame = adts_header(7 + payload.len()).unwrap().to_vec();
        frame.extend_from_slice(&payload);
        frame
    }
}
