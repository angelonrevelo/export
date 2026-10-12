//! Fixture recognizer: PCM → log-mel → one integer matmul → greedy CTC.
//!
//! The weight matrix is nine hand-checked integers, not a trained acoustic
//! model. A bin-centered 406.25 Hz tone becomes `H`. A bin-centered 1187.5 Hz
//! tone becomes `I`. Silence becomes the CTC blank. Anything else is whatever
//! those three logits happen to say — usually blank. This is not Whisper.

use crate::decode::{GreedyCtc, Transcript};
use crate::mel::{
    add_tone, quantize_peak, MelStream, HIGH_MEL, LOW_MEL, PCM_RATE, TONE_HIGH_HZ, TONE_LOW_HZ,
};
use crate::runtime::{matmul_i32, softmax_ppm};

/// Class ids: 0 blank, 1 `H`, 2 `I`. `vocab[0]` is unused by the collapse.
pub const VOCAB: [char; 3] = [' ', 'H', 'I'];

/// Row-major `3×3`. Row `t` is the weight of feature `t` onto `[blank, H, I]`.
/// Features are `[low_band, high_band, 1000]`.
///
/// `blank = -low - high`, `H = low - high`, `I = -low + high`.
pub const W: [i32; 9] = [-1, 1, -1, -1, -1, 1, 0, 0, 0];

/// Two tones, `H` then `I`, with silence around them. 1.2 s at 16 kHz.
pub fn fixture_hi_pcm() -> Vec<i16> {
    two_tone(TONE_LOW_HZ, TONE_HIGH_HZ)
}

/// Same schedule as [`fixture_hi_pcm`] with caller-chosen frequencies.
pub fn two_tone(first_hz: f32, second_hz: f32) -> Vec<i16> {
    let n = PCM_RATE as usize * 12 / 10;
    let mut pcm = vec![0i16; n];
    add_tone(&mut pcm, 0.25, 0.30, first_hz, 12_000);
    add_tone(&mut pcm, 0.70, 0.30, second_hz, 12_000);
    pcm
}

/// Run the fixture. `language` stays `None`: nothing here detects a language.
/// Working set is one [`MelStream`] plus the CTC token list.
pub fn transcribe_fixture(pcm: &[i16]) -> Transcript {
    let mut mel = MelStream::new();
    let mut dec = GreedyCtc::new(0, &VOCAB);
    mel.push_pcm(pcm, |index, frame| {
        let low = quantize_peak(frame, LOW_MEL.clone());
        let high = quantize_peak(frame, HIGH_MEL.clone());
        let x = [low, high, 1000];
        let mut logits = [0i32; 3];
        matmul_i32(&x, &W, &mut logits, 1, 3, 3).expect("3×3 fixture matmul");
        let mut ppm = [0i32; 3];
        softmax_ppm(&logits, &mut ppm).expect("3-class softmax");
        let class_id = if logits[1] > logits[0] && logits[1] >= logits[2] {
            1
        } else if logits[2] > logits[0] && logits[2] > logits[1] {
            2
        } else {
            0
        };
        let t = index as f32 * crate::mel::HOP as f32 / PCM_RATE as f32;
        let conf = ppm[class_id];
        dec.push(class_id, t, conf);
    });
    dec.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_empty() {
        let pcm = vec![0i16; PCM_RATE as usize];
        let t = transcribe_fixture(&pcm);
        assert_eq!(t.plain(), "");
        assert!(t.words.is_empty());
        assert!(t.language.is_none());
    }

    #[test]
    fn fixture_pcm_decodes_hi() {
        let t = transcribe_fixture(&fixture_hi_pcm());
        assert_eq!(t.plain(), "HI", "words={:?}", t.words);
        assert_eq!(t.words.len(), 2);
        assert_eq!(t.words[0].text, "H");
        assert_eq!(t.words[1].text, "I");
        assert!(t.words[0].end <= t.words[1].start + 0.05);
        assert!(t.words[0].confidence.unwrap() > 0.5);
        assert!(t.words[1].confidence.unwrap() > 0.5);
        assert!(t.language.is_none());
    }

    #[test]
    fn swapped_tones_decode_ih() {
        let t = transcribe_fixture(&two_tone(TONE_HIGH_HZ, TONE_LOW_HZ));
        assert_eq!(t.plain(), "IH", "words={:?}", t.words);
    }
}
