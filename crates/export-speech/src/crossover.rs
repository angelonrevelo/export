//! Butterworth crossover. Not stem separation.
//!
//! One lowpass biquad and one highpass biquad at 300 Hz, direct form II.
//! State is the four delay cells. `low` and `high` are written into caller
//! buffers. There is no vocal, drum, bass, or "other" model and no Demucs
//! weight file.
//!
//! A one-pole residual (`x - lowpass(x)`) was measured and dropped: at 80 Hz
//! the residual still held about a quarter of the low-band energy, so the
//! split was inside the noise of a useful crossover. The biquad is the
//! version that actually parks an 80 Hz sine in `low` and a 4 kHz sine in
//! `high`.

use crate::denoise::AudioError;
use std::f32::consts::TAU;

/// Default split used by [`Crossover::speech_bands`].
pub const CUTOFF_HZ: f32 = 300.0;

#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn lowpass(sample_rate: f32, cutoff: f32) -> Self {
        let (b0, b1, b2, a0, a1, a2) = butter(sample_rate, cutoff, true);
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    fn highpass(sample_rate: f32, cutoff: f32) -> Self {
        let (b0, b1, b2, a0, a1, a2) = butter(sample_rate, cutoff, false);
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    fn normalized(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

fn butter(sample_rate: f32, cutoff: f32, lowpass: bool) -> (f32, f32, f32, f32, f32, f32) {
    let w0 = TAU * cutoff / sample_rate;
    let cos = w0.cos();
    let sin = w0.sin();
    let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
    let (b0, b1, b2) = if lowpass {
        ((1.0 - cos) * 0.5, 1.0 - cos, (1.0 - cos) * 0.5)
    } else {
        ((1.0 + cos) * 0.5, -(1.0 + cos), (1.0 + cos) * 0.5)
    };
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos;
    let a2 = 1.0 - alpha;
    (b0, b1, b2, a0, a1, a2)
}

/// Streaming 300 Hz split. The struct is the filter state, not the audio.
#[derive(Clone, Copy, Debug)]
pub struct Crossover {
    low: Biquad,
    high: Biquad,
}

impl Crossover {
    pub fn new(sample_rate: u32, cutoff_hz: f32) -> Result<Self, AudioError> {
        if sample_rate == 0
            || cutoff_hz.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || cutoff_hz >= sample_rate as f32 * 0.5
        {
            return Err(AudioError::Rate);
        }
        let rate = sample_rate as f32;
        Ok(Self {
            low: Biquad::lowpass(rate, cutoff_hz),
            high: Biquad::highpass(rate, cutoff_hz),
        })
    }

    /// 300 Hz split at `sample_rate`.
    pub fn speech_bands(sample_rate: u32) -> Result<Self, AudioError> {
        Self::new(sample_rate, CUTOFF_HZ)
    }

    /// Write `low` and `high`. All three slices must be the same length.
    /// Does not allocate.
    pub fn process_into(
        &mut self,
        pcm: &[i16],
        low: &mut [i16],
        high: &mut [i16],
    ) -> Result<(), AudioError> {
        if low.len() != pcm.len() || high.len() != pcm.len() {
            return Err(AudioError::Shape);
        }
        for i in 0..pcm.len() {
            let x = pcm[i] as f32;
            let lo = self.low.process(x);
            let hi = self.high.process(x);
            low[i] = lo.clamp(-32_768.0, 32_767.0) as i16;
            high[i] = hi.clamp(-32_768.0, 32_767.0) as i16;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mel::PCM_RATE;
    use crate::tts::rms;
    use std::f32::consts::TAU;

    fn sine(freq: f32) -> Vec<i16> {
        let n = PCM_RATE as usize / 2;
        let mut pcm = vec![0i16; n];
        for (i, s) in pcm.iter_mut().enumerate() {
            let t = i as f32 / PCM_RATE as f32;
            *s = ((TAU * freq * t).sin() * 10_000.0) as i16;
        }
        pcm
    }

    #[test]
    fn state_is_tiny_and_bands_follow_frequency() {
        assert!(std::mem::size_of::<Crossover>() <= 128);
        let pcm_low = sine(80.0);
        let pcm_high = sine(4_000.0);
        let mut cross = Crossover::speech_bands(PCM_RATE).unwrap();
        let mut low = vec![0i16; pcm_low.len()];
        let mut high = vec![0i16; pcm_low.len()];
        cross.process_into(&pcm_low, &mut low, &mut high).unwrap();
        let skip = 800;
        let lo = rms(&low[skip..]);
        let hi = rms(&high[skip..]);
        assert!(lo > hi * 8.0, "80 Hz low {lo} high {hi}");

        cross = Crossover::speech_bands(PCM_RATE).unwrap();
        cross.process_into(&pcm_high, &mut low, &mut high).unwrap();
        let lo = rms(&low[skip..]);
        let hi = rms(&high[skip..]);
        assert!(hi > lo * 8.0, "4 kHz low {lo} high {hi}");
    }
}
