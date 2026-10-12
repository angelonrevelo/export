//! Streaming log-mel frontend.
//!
//! One [`MelStream`] holds a fixed ring, a Hann window, a dense HTK mel
//! filterbank, and one output frame. Pushing more PCM does not grow that
//! state. Frames are handed to the caller; this type does not keep them.
//!
//! 16 kHz, 512-point FFT, hop 160 (10 ms), 80 mels from 0 to 8 kHz.
//! That is the same rate and hop Whisper uses. It is not Whisper's model.

use crate::fft::fft;
use std::f32::consts::TAU;
use std::ops::Range;

/// PCM rate the frontend, the fixture recognizer, TTS, and denoise share.
pub const PCM_RATE: u32 = 16_000;
pub const N_FFT: usize = 512;
pub const HOP: usize = 160;
pub const N_MELS: usize = 80;
pub const N_FREQS: usize = N_FFT / 2 + 1;
pub const N_FILTERS: usize = N_MELS * N_FREQS;

/// Mel rows that a 406.25 Hz bin-centered tone lights up (FFT bin 13).
pub const LOW_MEL: Range<usize> = 6..22;
/// Mel rows that a 1187.5 Hz bin-centered tone lights up (FFT bin 38).
pub const HIGH_MEL: Range<usize> = 26..44;

/// Bin-centered tones used by the fixture recognizer. `k * rate / n_fft`.
pub const TONE_LOW_HZ: f32 = 13.0 * PCM_RATE as f32 / N_FFT as f32;
pub const TONE_HIGH_HZ: f32 = 38.0 * PCM_RATE as f32 / N_FFT as f32;

/// Log-mel stream. About 90 KiB, independent of how much audio has been pushed.
pub struct MelStream {
    ring: [f32; N_FFT],
    pos: usize,
    seen: u32,
    re: [f32; N_FFT],
    im: [f32; N_FFT],
    hann: [f32; N_FFT],
    filters: [f32; N_FILTERS],
    frame: [f32; N_MELS],
}

impl Default for MelStream {
    fn default() -> Self {
        Self::new()
    }
}

impl MelStream {
    pub fn new() -> Self {
        let mut stream = Self {
            ring: [0.0; N_FFT],
            pos: 0,
            seen: 0,
            re: [0.0; N_FFT],
            im: [0.0; N_FFT],
            hann: [0.0; N_FFT],
            filters: [0.0; N_FILTERS],
            frame: [0.0; N_MELS],
        };
        for i in 0..N_FFT {
            stream.hann[i] = 0.5 * (1.0 - (TAU * i as f32 / N_FFT as f32).cos());
        }
        build_filters(&mut stream.filters);
        stream
    }

    /// Drop history. Does not allocate.
    pub fn reset(&mut self) {
        self.ring = [0.0; N_FFT];
        self.pos = 0;
        self.seen = 0;
    }

    /// Push PCM. `emit(frame_index, log_mel)` runs once per hop after the
    /// first full window. `frame_index` starts at 0. The slice is valid only
    /// for the duration of the callback.
    pub fn push_pcm(&mut self, samples: &[i16], mut emit: impl FnMut(u32, &[f32; N_MELS])) {
        for &sample in samples {
            self.ring[self.pos] = sample as f32 * (1.0 / 32768.0);
            self.pos += 1;
            if self.pos == N_FFT {
                self.pos = 0;
            }
            self.seen += 1;
            if self.seen >= N_FFT as u32 {
                let past = self.seen - N_FFT as u32;
                if past % HOP as u32 == 0 {
                    let index = past / HOP as u32;
                    self.analyze();
                    emit(index, &self.frame);
                }
            }
        }
    }

    fn analyze(&mut self) {
        for i in 0..N_FFT {
            let s = self.ring[(self.pos + i) % N_FFT] * self.hann[i];
            self.re[i] = s;
            self.im[i] = 0.0;
        }
        let ok = fft(&mut self.re, &mut self.im);
        debug_assert!(ok);
        for m in 0..N_MELS {
            let row = m * N_FREQS;
            let mut acc = 0.0f32;
            for k in 0..N_FREQS {
                let w = self.filters[row + k];
                if w != 0.0 {
                    let p = self.re[k] * self.re[k] + self.im[k] * self.im[k];
                    acc += w * p;
                }
            }
            self.frame[m] = acc.max(1.0e-10).ln();
        }
    }
}

/// Peak log-mel in `bins`, scaled by 1000 and truncated to `i32`.
///
/// A sum of logs is dominated by the quiet bins in the band, so a pure tone
/// looks like silence. The peak is the bin the tone actually lights.
pub fn quantize_peak(frame: &[f32], bins: Range<usize>) -> i32 {
    let mut peak = f32::NEG_INFINITY;
    for v in frame.iter().take(bins.end).skip(bins.start) {
        if *v > peak {
            peak = *v;
        }
    }
    if !peak.is_finite() {
        return 0;
    }
    (peak * 1000.0).clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

/// Index of the largest log-mel bin.
pub fn peak_bin(frame: &[f32]) -> usize {
    let mut best_i = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in frame.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best_i = i;
        }
    }
    best_i
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

fn build_filters(out: &mut [f32; N_FILTERS]) {
    let mel_min = hz_to_mel(0.0);
    let mel_max = hz_to_mel(8_000.0);
    let mut points = [0.0f32; N_MELS + 2];
    for (i, point) in points.iter_mut().enumerate() {
        let mel = mel_min + (mel_max - mel_min) * (i as f32) / (N_MELS + 1) as f32;
        let hz = mel_to_hz(mel);
        *point = ((N_FFT + 1) as f32 * hz / PCM_RATE as f32).floor();
    }
    for m in 0..N_MELS {
        let left = points[m];
        let center = points[m + 1];
        let right = points[m + 2];
        for k in 0..N_FREQS {
            let kf = k as f32;
            let w = if center <= left || right <= center || kf <= left || kf >= right {
                0.0
            } else if kf <= center {
                (kf - left) / (center - left)
            } else {
                (right - kf) / (right - center)
            };
            out[m * N_FREQS + k] = w;
        }
    }
}

/// Sine at `freq_hz` added into `pcm` over `[start_sec, start_sec+dur_sec)`.
pub fn add_tone(pcm: &mut [i16], start_sec: f32, dur_sec: f32, freq_hz: f32, amp: i16) {
    let start = (start_sec * PCM_RATE as f32) as usize;
    let dur = (dur_sec * PCM_RATE as f32) as usize;
    let end = (start + dur).min(pcm.len());
    for (n, sample) in pcm.iter_mut().enumerate().take(end).skip(start) {
        let t = n as f32 / PCM_RATE as f32;
        let s = (TAU * freq_hz * t).sin();
        *sample = (s * f32::from(amp)) as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, secs: f32) -> Vec<i16> {
        let n = (secs * PCM_RATE as f32) as usize;
        let mut pcm = vec![0i16; n];
        add_tone(&mut pcm, 0.0, secs, freq, 12_000);
        pcm
    }

    #[test]
    fn silence_sits_on_the_log_floor() {
        let mut mel = MelStream::new();
        let pcm = vec![0i16; PCM_RATE as usize];
        let mut saw = 0u32;
        mel.push_pcm(&pcm, |_i, frame| {
            saw += 1;
            for v in frame {
                assert!(*v < -20.0, "silence bin {v}");
            }
        });
        assert!(saw > 50, "frames {saw}");
    }

    #[test]
    fn low_tone_peaks_in_low_band() {
        let pcm = tone(TONE_LOW_HZ, 0.5);
        let mut mel = MelStream::new();
        let mut peaks = Vec::new();
        mel.push_pcm(&pcm, |idx, frame| {
            if (8..30).contains(&idx) {
                peaks.push(peak_bin(frame));
            }
        });
        assert!(!peaks.is_empty());
        for p in &peaks {
            assert!(
                LOW_MEL.contains(p),
                "low tone peaked at mel {p}, want {LOW_MEL:?}"
            );
        }
    }

    #[test]
    fn high_tone_peaks_in_high_band() {
        let pcm = tone(TONE_HIGH_HZ, 0.5);
        let mut mel = MelStream::new();
        let mut peaks = Vec::new();
        mel.push_pcm(&pcm, |idx, frame| {
            if (8..30).contains(&idx) {
                peaks.push(peak_bin(frame));
            }
        });
        assert!(!peaks.is_empty());
        for p in &peaks {
            assert!(
                HIGH_MEL.contains(p),
                "high tone peaked at mel {p}, want {HIGH_MEL:?}"
            );
        }
    }

    #[test]
    fn state_does_not_grow_with_audio() {
        let mut mel = MelStream::new();
        assert!(std::mem::size_of_val(&mel) < 128 * 1024);
        let pcm = vec![0i16; PCM_RATE as usize * 4];
        let mut n = 0u32;
        mel.push_pcm(&pcm, |_, _| n += 1);
        let bytes = std::mem::size_of_val(&mel);
        mel.push_pcm(&pcm, |_, _| n += 1);
        assert_eq!(std::mem::size_of_val(&mel), bytes);
        assert!(n > 200);
    }

    #[test]
    fn reset_matches_a_fresh_stream() {
        let pcm = tone(TONE_LOW_HZ, 0.3);
        let mut a = MelStream::new();
        let mut b = MelStream::new();
        let mut frames_a = Vec::new();
        a.push_pcm(&pcm, |_, f| frames_a.push(*f));
        a.push_pcm(&pcm, |_, _| {});
        a.reset();
        let mut frames_b = Vec::new();
        b.push_pcm(&pcm, |_, f| frames_b.push(*f));
        let mut frames_a2 = Vec::new();
        a.push_pcm(&pcm, |_, f| frames_a2.push(*f));
        assert_eq!(frames_a.len(), frames_b.len());
        assert_eq!(frames_a, frames_a2);
    }
}
