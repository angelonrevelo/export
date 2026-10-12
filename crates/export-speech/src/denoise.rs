//! Spectral subtraction with a fixed scratch buffer.
//!
//! FFT length 256, hop 128. The noise floor is the mean power of frames that
//! start inside the first 2048 samples, then frozen. Later frames are scaled
//! per bin by `max(1 - noise/power, 0.2)` and overlap-added with a periodic
//! Hann window. Scratch is a few kilobytes. The output buffer is the audio
//! itself, not a stored spectrogram.
//!
//! Limit: a clip whose first 128 ms is already speech gets that speech into
//! the noise estimate and comes back dulled. This is not a neural denoiser
//! and it is not `noisereduce`.

use crate::fft::{fft, ifft};
use crate::wav::write_wav;
use std::f32::consts::TAU;

const FFT_N: usize = 256;
const HOP: usize = 128;
const N_FREQ: usize = FFT_N / 2 + 1;
/// Frames that start before this sample contribute to the noise floor.
const NOISE_LEN: usize = 2048;
const GAIN_FLOOR: f32 = 0.2;

/// Why denoise refused the buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    Shape,
    Rate,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Shape => write!(f, "buffer length"),
            AudioError::Rate => write!(f, "sample rate"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Reusable FFT / overlap-add state. Construct once, run many clips.
pub struct DenoiseScratch {
    hann: [f32; FFT_N],
    re: [f32; FFT_N],
    im: [f32; FFT_N],
    acc: [f32; FFT_N],
    noise: [f32; N_FREQ],
    noise_n: u32,
}

impl Default for DenoiseScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl DenoiseScratch {
    pub fn new() -> Self {
        let mut scratch = Self {
            hann: [0.0; FFT_N],
            re: [0.0; FFT_N],
            im: [0.0; FFT_N],
            acc: [0.0; FFT_N],
            noise: [0.0; N_FREQ],
            noise_n: 0,
        };
        for i in 0..FFT_N {
            scratch.hann[i] = 0.5 * (1.0 - (TAU * i as f32 / FFT_N as f32).cos());
        }
        scratch
    }

    /// Overlap-add the windowed signal back onto itself. No spectral change.
    /// Used to prove the COLA path before subtraction is trusted.
    pub fn identity_into(&mut self, pcm: &[i16], out: &mut [i16]) -> Result<(), AudioError> {
        self.ola(pcm, out, false)
    }

    /// Estimate a noise floor from the start of `pcm`, then subtract it.
    pub fn denoise_into(&mut self, pcm: &[i16], out: &mut [i16]) -> Result<(), AudioError> {
        self.estimate(pcm);
        self.ola(pcm, out, true)
    }

    /// PCM in, WAV out. One allocation for the file (and the `i16` body inside it).
    pub fn denoise_wav(&mut self, pcm: &[i16], sample_rate: u32) -> Result<Vec<u8>, AudioError> {
        if sample_rate == 0 {
            return Err(AudioError::Rate);
        }
        let mut body = vec![0i16; pcm.len()];
        self.denoise_into(pcm, &mut body)?;
        Ok(write_wav(&body, sample_rate))
    }

    fn estimate(&mut self, pcm: &[i16]) {
        self.noise = [0.0; N_FREQ];
        self.noise_n = 0;
        let mut start = 0usize;
        while start < NOISE_LEN && start < pcm.len() {
            self.load(pcm, start);
            let ok = fft(&mut self.re, &mut self.im);
            debug_assert!(ok);
            for k in 0..N_FREQ {
                self.noise[k] += self.re[k] * self.re[k] + self.im[k] * self.im[k];
            }
            self.noise_n += 1;
            start += HOP;
        }
        if self.noise_n > 0 {
            let n = self.noise_n as f32;
            for bin in &mut self.noise {
                *bin /= n;
            }
        }
    }

    fn load(&mut self, pcm: &[i16], start: usize) {
        for i in 0..FFT_N {
            let idx = start + i;
            let s = if idx < pcm.len() {
                pcm[idx] as f32 * (1.0 / 32_768.0)
            } else {
                0.0
            };
            self.re[i] = s * self.hann[i];
            self.im[i] = 0.0;
        }
    }

    fn ola(&mut self, pcm: &[i16], out: &mut [i16], subtract: bool) -> Result<(), AudioError> {
        if out.len() != pcm.len() {
            return Err(AudioError::Shape);
        }
        self.acc = [0.0; FFT_N];
        let mut start = 0usize;
        while start < pcm.len() {
            self.load(pcm, start);
            let ok = fft(&mut self.re, &mut self.im);
            debug_assert!(ok);
            if subtract {
                self.apply_gain();
            }
            let ok = ifft(&mut self.re, &mut self.im);
            debug_assert!(ok);
            for i in 0..FFT_N {
                self.acc[i] += self.re[i];
            }
            let n_emit = HOP.min(pcm.len() - start);
            for i in 0..n_emit {
                let q = (self.acc[i] * 32_768.0).clamp(-32_768.0, 32_767.0);
                out[start + i] = q as i16;
            }
            self.acc.copy_within(HOP..FFT_N, 0);
            for slot in &mut self.acc[FFT_N - HOP..] {
                *slot = 0.0;
            }
            start += HOP;
        }
        Ok(())
    }

    fn apply_gain(&mut self) {
        for k in 0..N_FREQ {
            let power = self.re[k] * self.re[k] + self.im[k] * self.im[k];
            let gain = (1.0 - self.noise[k] / (power + 1.0e-8)).max(GAIN_FLOOR);
            self.re[k] *= gain;
            self.im[k] *= gain;
        }
        for k in 1..FFT_N / 2 {
            self.re[FFT_N - k] = self.re[k];
            self.im[FFT_N - k] = -self.im[k];
        }
        self.im[0] = 0.0;
        self.im[FFT_N / 2] = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mel::PCM_RATE;
    use crate::tts::rms;
    use crate::wav::parse_wav;
    use std::f32::consts::TAU;

    fn hash_noise(i: usize) -> f32 {
        let mut x = (i as u32)
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        x ^= x >> 16;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn corr(a: &[i16], b: &[i16]) -> f32 {
        let mut dot = 0.0f64;
        let mut na = 0.0f64;
        let mut nb = 0.0f64;
        for (x, y) in a.iter().zip(b) {
            let xf = f64::from(*x);
            let yf = f64::from(*y);
            dot += xf * yf;
            na += xf * xf;
            nb += yf * yf;
        }
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        (dot / (na.sqrt() * nb.sqrt())) as f32
    }

    #[test]
    fn scratch_is_small_and_identity_holds_in_the_middle() {
        let scratch = DenoiseScratch::new();
        assert!(std::mem::size_of_val(&scratch) < 16 * 1024);
        let n = PCM_RATE as usize;
        let mut pcm = vec![0i16; n];
        for (i, s) in pcm.iter_mut().enumerate() {
            let t = i as f32 / PCM_RATE as f32;
            *s = ((TAU * 406.25 * t).sin() * 8_000.0) as i16;
        }
        let mut out = vec![0i16; n];
        let mut scratch = scratch;
        scratch.identity_into(&pcm, &mut out).unwrap();
        let c = corr(&pcm[2_000..14_000], &out[2_000..14_000]);
        assert!(c > 0.98, "identity correlation {c}");
    }

    #[test]
    fn noise_preamble_drops_and_sine_stays() {
        let n = PCM_RATE as usize;
        let mut pcm = vec![0i16; n];
        for (i, s) in pcm.iter_mut().enumerate() {
            let noise = (hash_noise(i) * 800.0) as i16;
            let tone = if i > 4_000 {
                let t = i as f32 / PCM_RATE as f32;
                ((TAU * 406.25 * t).sin() * 8_000.0) as i16
            } else {
                0
            };
            *s = tone.saturating_add(noise);
        }
        let mut out = vec![0i16; n];
        let mut scratch = DenoiseScratch::new();
        scratch.denoise_into(&pcm, &mut out).unwrap();
        let noise_in = rms(&pcm[512..1_500]);
        let noise_out = rms(&out[512..1_500]);
        assert!(
            noise_out < noise_in * 0.75,
            "noise rms {noise_out} vs {noise_in}"
        );
        let c = corr(&pcm[6_000..14_000], &out[6_000..14_000]);
        assert!(c > 0.9, "sine correlation {c}");
        let wav = scratch.denoise_wav(&pcm, PCM_RATE).unwrap();
        let parsed = parse_wav(&wav).unwrap();
        assert_eq!(parsed.pcm.len(), pcm.len());
        assert_eq!(parsed.sample_rate, PCM_RATE);
    }
}
