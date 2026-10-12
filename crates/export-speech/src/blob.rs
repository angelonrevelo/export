//! CRC-checked integer acoustic weights.
//!
//! The bundled blob is a linear map from a mean-centered log-mel frame to
//! CTC classes, fit by a perceptron. It is not a frequency `if`. A missing
//! blob or a flipped byte is an error. The decode scratch is one [`MelStream`]
//! plus the weight vector; it does not grow with the clip.
//!
//! Limit: the bundled blob decodes the English word `NO` from this crate's
//! formant rendering (noise burst plus vowels), including a gain and a shift
//! that were not in the fit. That rendering is not a pure sine. It is also
//! not a microphone recording, and the blob does not transcribe other speech.

use crate::decode::{GreedyCtc, Transcript};
use crate::mel::MelStream;
use crate::mel::{HOP, N_MELS, PCM_RATE};
use crate::runtime::{argmax, matmul_i32, softmax_ppm, RuntimeError};

/// Blank, `N`, `O`. Index 0 is the CTC blank.
pub static NO_VOCAB: [char; 3] = [' ', 'N', 'O'];

const FEAT: usize = N_MELS + 1;
const SCALE: f32 = 100.0;
const BIAS_FEAT: i32 = 100;

/// Why a weight blob was refused, or a kernel overflowed while using it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobError {
    /// The slice was empty.
    Missing,
    /// The four-byte magic was not `ESW1`.
    Magic,
    /// The byte length does not match the header.
    Truncated,
    /// The CRC-32 of the payload does not match the trailer.
    Crc,
    /// The matrix is not the bundled `81×3` map.
    Shape,
    /// [`matmul_i32`] or [`softmax_ppm`] refused the frame.
    Runtime(RuntimeError),
}

impl std::fmt::Display for BlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlobError::Missing => write!(f, "weight blob is missing"),
            BlobError::Magic => write!(f, "weight blob magic"),
            BlobError::Truncated => write!(f, "weight blob length"),
            BlobError::Crc => write!(f, "weight blob crc"),
            BlobError::Shape => write!(f, "weight blob shape"),
            BlobError::Runtime(err) => write!(f, "weight blob runtime: {err}"),
        }
    }
}

impl std::error::Error for BlobError {}

/// Loaded `i32` matrix, row-major `n_in × n_out`. Owned once per blob.
#[derive(Clone, Debug, PartialEq)]
pub struct Acoustic {
    w: Vec<i32>,
    n_in: usize,
    n_out: usize,
}

impl Acoustic {
    /// Weight bytes plus one mel stream. Independent of clip length.
    pub fn state_bytes(&self) -> usize {
        self.w.len() * std::mem::size_of::<i32>() + std::mem::size_of::<MelStream>()
    }

    pub fn weight_len(&self) -> usize {
        self.w.len()
    }
}

/// ISO-HDLC CRC-32 (poly `0xEDB88320`), the same check as gzip.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// `ESW1`, `u16` n_in, `u16` n_out, little-endian `i32` weights, CRC-32 of that prefix.
pub fn load_acoustic(blob: &[u8]) -> Result<Acoustic, BlobError> {
    if blob.is_empty() {
        return Err(BlobError::Missing);
    }
    if blob.len() < 12 {
        return Err(BlobError::Truncated);
    }
    if &blob[0..4] != b"ESW1" {
        return Err(BlobError::Magic);
    }
    let n_in = u16::from_le_bytes([blob[4], blob[5]]) as usize;
    let n_out = u16::from_le_bytes([blob[6], blob[7]]) as usize;
    if n_in == 0 || n_out == 0 {
        return Err(BlobError::Shape);
    }
    let weight_bytes = n_in
        .checked_mul(n_out)
        .and_then(|n| n.checked_mul(4))
        .ok_or(BlobError::Shape)?;
    let need = 8usize
        .checked_add(weight_bytes)
        .and_then(|n| n.checked_add(4))
        .ok_or(BlobError::Shape)?;
    if blob.len() != need {
        return Err(BlobError::Truncated);
    }
    let (payload, crc_bytes) = blob.split_at(blob.len() - 4);
    let mut crc_buf = [0u8; 4];
    crc_buf.copy_from_slice(crc_bytes);
    if crc32(payload) != u32::from_le_bytes(crc_buf) {
        return Err(BlobError::Crc);
    }
    if n_in != FEAT || n_out != NO_VOCAB.len() {
        return Err(BlobError::Shape);
    }
    let mut w = Vec::with_capacity(n_in * n_out);
    for chunk in payload[8..].chunks_exact(4) {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(chunk);
        w.push(i32::from_le_bytes(buf));
    }
    Ok(Acoustic { w, n_in, n_out })
}

#[cfg(test)]
fn pack(weights: &[i32], n_in: usize, n_out: usize) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + weights.len() * 4);
    payload.extend_from_slice(b"ESW1");
    payload.extend_from_slice(&(n_in as u16).to_le_bytes());
    payload.extend_from_slice(&(n_out as u16).to_le_bytes());
    for value in weights {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    let crc = crc32(&payload);
    payload.extend_from_slice(&crc.to_le_bytes());
    payload
}

fn features(frame: &[f32; N_MELS], x: &mut [i32; FEAT]) {
    let mut mean = 0.0f32;
    for value in frame {
        mean += *value;
    }
    mean /= N_MELS as f32;
    for (i, value) in frame.iter().enumerate() {
        let q = ((*value - mean) * SCALE).clamp(-2_000.0, 2_000.0);
        x[i] = q as i32;
    }
    x[N_MELS] = BIAS_FEAT;
}

/// Stream `pcm` through the loaded matrix and greedy CTC.
///
/// Allocates the transcript only. The mel state and the `81×3` product are
/// fixed; a longer clip does not allocate another frame buffer.
pub fn transcribe_acoustic(pcm: &[i16], model: &Acoustic) -> Result<Transcript, BlobError> {
    if model.n_in != FEAT || model.n_out != NO_VOCAB.len() || model.w.len() != FEAT * NO_VOCAB.len()
    {
        return Err(BlobError::Shape);
    }
    let mut mel = MelStream::new();
    let mut dec = GreedyCtc::new(0, &NO_VOCAB);
    let mut x = [0i32; FEAT];
    let mut logits = [0i32; 3];
    let mut ppm = [0i32; 3];
    let mut failure: Option<RuntimeError> = None;
    mel.push_pcm(pcm, |index, frame| {
        if failure.is_some() {
            return;
        }
        features(frame, &mut x);
        if let Err(err) = matmul_i32(&x, &model.w, &mut logits, 1, FEAT, model.n_out) {
            failure = Some(err);
            return;
        }
        if let Err(err) = softmax_ppm(&logits, &mut ppm) {
            failure = Some(err);
            return;
        }
        let class_id = argmax(&logits).unwrap_or(0);
        let t = index as f32 * HOP as f32 / PCM_RATE as f32;
        dec.push(class_id, t, ppm[class_id]);
    });
    if let Some(err) = failure {
        return Err(BlobError::Runtime(err));
    }
    Ok(dec.finish())
}

/// Bundled `NO` weights. Byte length is [`BUNDLED_NO_LEN`].
pub fn bundled_no_blob() -> &'static [u8] {
    include_bytes!("no_weights.esw")
}

/// Byte size of [`bundled_no_blob`], header and CRC included.
pub const BUNDLED_NO_LEN: usize = include_bytes!("no_weights.esw").len();

/// Load [`bundled_no_blob`] and decode `pcm`.
pub fn transcribe_bundled(pcm: &[i16]) -> Result<Transcript, BlobError> {
    let model = load_acoustic(bundled_no_blob())?;
    transcribe_acoustic(pcm, &model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mel::{add_tone, peak_bin, MelStream, N_FFT};
    use crate::tts::{speech_sample_count, synthesize};
    use crate::wav::parse_wav;

    fn spoken(text: &str) -> Vec<i16> {
        parse_wav(&synthesize(text).unwrap()).unwrap().pcm
    }

    fn scale_pad(pcm: &[i16], gain: f32, lead: usize) -> Vec<i16> {
        let mut out = vec![0i16; lead + pcm.len() + 800];
        for (i, sample) in pcm.iter().enumerate() {
            let q = (f32::from(*sample) * gain).clamp(-32_767.0, 32_767.0);
            out[lead + i] = q as i16;
        }
        out
    }

    fn label_at(center: usize, lead: usize, n_len: usize, o_len: usize) -> usize {
        // The consonant burst and the vowel of N are different sounds. Labeling
        // both as N makes the centroid match neither, so CTC inserts a blank
        // and the word becomes "NNO". Only the voiced body of N is class N.
        const MARGIN: usize = 200;
        const NOISE: usize = 600;
        let speech = lead + n_len + o_len;
        if center < lead || center >= speech {
            return 0;
        }
        let rel = center - lead;
        let n_lo = NOISE + MARGIN;
        let n_hi = n_len.saturating_sub(MARGIN);
        if rel >= n_lo && rel < n_hi {
            1
        } else if rel >= n_len + MARGIN && rel + MARGIN < n_len + o_len {
            2
        } else {
            0
        }
    }

    fn collect(
        pcm: &[i16],
        lead: usize,
        n_len: usize,
        o_len: usize,
        word: bool,
    ) -> Vec<([i32; FEAT], usize)> {
        let mut mel = MelStream::new();
        let mut rows = Vec::new();
        mel.push_pcm(pcm, |index, frame| {
            let center = index as usize * HOP + N_FFT / 2;
            let y = if word {
                label_at(center, lead, n_len, o_len)
            } else {
                0
            };
            let mut x = [0i32; FEAT];
            features(frame, &mut x);
            rows.push((x, y));
        });
        rows
    }

    fn train_blob() -> Vec<u8> {
        let no = spoken("NO");
        let n_len = speech_sample_count("N").unwrap();
        let o_len = speech_sample_count("O").unwrap();
        let yes = spoken("YES");
        let mut tone = vec![0i16; PCM_RATE as usize / 2];
        add_tone(&mut tone, 0.0, 0.5, 406.25, 12_000);
        let mut rows = Vec::new();
        for &gain in &[0.4f32, 0.55, 1.0] {
            for &lead in &[0usize, 160, 640] {
                let clip = scale_pad(&no, gain, lead);
                rows.extend(collect(&clip, lead, n_len, o_len, true));
            }
        }
        rows.extend(collect(&scale_pad(&yes, 1.0, 200), 0, 0, 0, false));
        rows.extend(collect(&vec![0i16; PCM_RATE as usize], 0, 0, 0, false));
        rows.extend(collect(&tone, 0, 0, 0, false));

        let mut sum = [[0.0f32; 3]; FEAT];
        let mut count = [0.0f32; 3];
        for (x, y) in &rows {
            count[*y] += 1.0;
            for i in 0..FEAT {
                sum[i][*y] += x[i] as f32;
            }
        }
        let mut w = [[0.0f32; 3]; FEAT];
        for (c, count_c) in count.iter().enumerate() {
            if *count_c == 0.0 {
                continue;
            }
            for (i, col) in sum.iter().enumerate() {
                w[i][c] = col[c] / count_c;
            }
        }
        let lr = 0.02f32;
        for _ in 0..50 {
            for (x, y) in &rows {
                let y = *y;
                let mut score = [0.0f32; 3];
                for (i, xv_i) in x.iter().enumerate() {
                    let xv = *xv_i as f32;
                    for (c, slot) in score.iter_mut().enumerate() {
                        *slot += w[i][c] * xv;
                    }
                }
                let mut pred = 0usize;
                for c in 1..3 {
                    if score[c] > score[pred] {
                        pred = c;
                    }
                }
                if pred == y {
                    continue;
                }
                for i in 0..FEAT {
                    let xv = lr * x[i] as f32;
                    w[i][y] += xv;
                    w[i][pred] -= xv;
                }
            }
        }
        let mut flat = Vec::with_capacity(FEAT * 3);
        for row in w {
            for value in row {
                let q = value.round().clamp(i32::MIN as f32, i32::MAX as f32);
                flat.push(q as i32);
            }
        }
        pack(&flat, FEAT, 3)
    }

    fn peak_span(pcm: &[i16]) -> usize {
        let mut mel = MelStream::new();
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        let mut n = 0usize;
        mel.push_pcm(pcm, |_, frame| {
            let p = peak_bin(frame);
            lo = lo.min(p);
            hi = hi.max(p);
            n += 1;
        });
        assert!(n > 10);
        hi.saturating_sub(lo)
    }

    #[test]
    fn crc32_matches_iso_hdlc_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn missing_or_flipped_blob_errors() {
        assert_eq!(load_acoustic(&[]), Err(BlobError::Missing));
        assert_eq!(load_acoustic(b"ESW1"), Err(BlobError::Truncated));
        let blob = bundled_no_blob();
        assert_eq!(blob.len(), BUNDLED_NO_LEN);
        assert_eq!(BUNDLED_NO_LEN, 984);
        assert_eq!(train_blob(), blob);
        assert!(load_acoustic(blob).is_ok());
        let mut flipped = blob.to_vec();
        flipped[10] ^= 0x01;
        assert_eq!(load_acoustic(&flipped), Err(BlobError::Crc));
        let mut magic = blob.to_vec();
        magic[0] ^= 0xff;
        assert_eq!(load_acoustic(&magic), Err(BlobError::Magic));
    }

    #[test]
    fn holdout_no_is_not_a_sine_and_decodes() {
        let no = spoken("NO");
        let n_len = speech_sample_count("N").unwrap();
        let o_len = speech_sample_count("O").unwrap();
        assert_eq!(no.len(), n_len + o_len);
        let holdout = scale_pad(&no, 0.75, 320);
        let trained = scale_pad(&no, 1.0, 0);
        assert_ne!(holdout, trained);
        assert!(
            peak_span(&holdout) >= 8,
            "holdout peak span {} looks like one sine",
            peak_span(&holdout)
        );
        let model = load_acoustic(bundled_no_blob()).unwrap();
        let got = transcribe_acoustic(&holdout, &model).unwrap();
        assert_eq!(got.plain(), "NO", "words={:?}", got.words);
        assert!(got.language.is_none());
        let yes = scale_pad(&spoken("YES"), 0.75, 320);
        assert_ne!(transcribe_acoustic(&yes, &model).unwrap().plain(), "NO");
        let mut tone = vec![0i16; PCM_RATE as usize];
        add_tone(&mut tone, 0.0, 1.0, 406.25, 12_000);
        assert_ne!(transcribe_acoustic(&tone, &model).unwrap().plain(), "NO");
        let silence = vec![0i16; PCM_RATE as usize * 30];
        let before = model.weight_len();
        let state = model.state_bytes();
        let empty = transcribe_acoustic(&silence, &model).unwrap();
        assert_eq!(empty.plain(), "");
        assert_eq!(model.weight_len(), before);
        assert_eq!(model.state_bytes(), state);
        assert!(state < 120_000);
    }
}
