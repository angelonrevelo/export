//! Deterministic formant synthesizer for short ASCII.
//!
//! Each letter is one or two fixed phones (noise burst and/or three sines at
//! fixed formant frequencies). Space is silence. The same string always
//! produces the same WAV. There is no recorded diphone inventory and no
//! neural vocoder. It is not Piper and it is not macOS `say`.

use crate::mel::PCM_RATE;
use crate::wav::write_wav;
use std::f32::consts::TAU;

/// Why synthesis stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthError {
    /// Empty input.
    Empty,
    /// A byte outside printable ASCII (space through `~`).
    NonAscii,
}

impl std::fmt::Display for SynthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SynthError::Empty => write!(f, "empty text"),
            SynthError::NonAscii => write!(f, "text is not printable ASCII"),
        }
    }
}

impl std::error::Error for SynthError {}

#[derive(Clone, Copy)]
enum Kind {
    Silence,
    Voiced,
    Noise,
}

#[derive(Clone, Copy)]
struct Phone {
    kind: Kind,
    f1: f32,
    f2: f32,
    f3: f32,
    n: usize,
    amp: f32,
}

const VOWEL: usize = 2_400;
const NOISE: usize = 600;
const BODY: usize = 1_800;
const GAP: usize = 1_600;

fn voiced(f1: f32, f2: f32, f3: f32, n: usize, amp: f32) -> Phone {
    Phone {
        kind: Kind::Voiced,
        f1,
        f2,
        f3,
        n,
        amp,
    }
}

fn noise(n: usize, amp: f32) -> Phone {
    Phone {
        kind: Kind::Noise,
        f1: 1_400.0,
        f2: 0.0,
        f3: 0.0,
        n,
        amp,
    }
}

fn silence(n: usize) -> Phone {
    Phone {
        kind: Kind::Silence,
        f1: 0.0,
        f2: 0.0,
        f3: 0.0,
        n,
        amp: 0.0,
    }
}

/// At most two phones. Returns how many of `out` were filled.
fn phones_for(ch: char, out: &mut [Phone; 2]) -> usize {
    let vowel = |f1, f2, f3| voiced(f1, f2, f3, VOWEL, 0.28);
    match ch {
        'A' | 'a' => {
            out[0] = vowel(700.0, 1_200.0, 2_500.0);
            1
        }
        'E' | 'e' => {
            out[0] = vowel(450.0, 2_000.0, 2_600.0);
            1
        }
        'I' | 'i' => {
            out[0] = vowel(300.0, 2_300.0, 3_000.0);
            1
        }
        'O' | 'o' => {
            out[0] = vowel(500.0, 900.0, 2_400.0);
            1
        }
        'U' | 'u' => {
            out[0] = vowel(320.0, 800.0, 2_200.0);
            1
        }
        ' ' => {
            out[0] = silence(GAP);
            1
        }
        '0'..='9' => {
            let d = (ch as u8 - b'0') as f32;
            out[0] = voiced(300.0 + 40.0 * d, 1_400.0, 2_600.0, VOWEL, 0.26);
            1
        }
        c if c.is_ascii_graphic() => {
            let fold = c.to_ascii_uppercase() as u8;
            let f1 = 400.0 + f32::from(fold % 7) * 30.0;
            let f2 = 1_200.0 + f32::from(fold % 11) * 40.0;
            out[0] = noise(NOISE, 0.22);
            out[1] = voiced(f1, f2, 2_500.0, BODY, 0.24);
            2
        }
        _ => 0,
    }
}

fn check(text: &str) -> Result<(), SynthError> {
    if text.is_empty() {
        return Err(SynthError::Empty);
    }
    if text.chars().any(|c| !c.is_ascii() || c.is_ascii_control()) {
        return Err(SynthError::NonAscii);
    }
    Ok(())
}

/// Sample count the synthesizer will emit for `text`.
pub fn speech_sample_count(text: &str) -> Result<usize, SynthError> {
    check(text)?;
    let mut n = 0usize;
    let mut buf = [silence(0); 2];
    for ch in text.chars() {
        let k = phones_for(ch, &mut buf);
        for phone in buf.iter().take(k) {
            n += phone.n;
        }
    }
    Ok(n)
}

struct Rng {
    state: u32,
}

impl Rng {
    fn next(&mut self) -> f32 {
        self.state = self
            .state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let unit = (self.state >> 8) as f32 / 16_777_216.0;
        unit * 2.0 - 1.0
    }
}

fn render_phone(phone: &Phone, pcm: &mut Vec<i16>, phases: &mut [f32; 3], rng: &mut Rng) {
    let gains = [1.0f32, 0.55, 0.28];
    let freqs = [phone.f1, phone.f2, phone.f3];
    let fade = 80usize.min(phone.n / 2);
    for i in 0..phone.n {
        let env = if fade == 0 {
            1.0
        } else if i < fade {
            i as f32 / fade as f32
        } else if i + fade >= phone.n {
            (phone.n - i) as f32 / fade as f32
        } else {
            1.0
        };
        let sample = match phone.kind {
            Kind::Silence => 0.0,
            Kind::Noise => rng.next() * phone.amp * env,
            Kind::Voiced => {
                let mut y = 0.0f32;
                for k in 0..3 {
                    phases[k] += TAU * freqs[k] / PCM_RATE as f32;
                    if phases[k] > TAU {
                        phases[k] -= TAU;
                    }
                    y += gains[k] * phases[k].sin();
                }
                y * phone.amp * env
            }
        };
        let q = (sample * 28_000.0).clamp(-32_767.0, 32_767.0);
        pcm.push(q as i16);
    }
}

/// PCM for `text`, then a mono 16-bit WAV at [`PCM_RATE`].
pub fn synthesize(text: &str) -> Result<Vec<u8>, SynthError> {
    let n = speech_sample_count(text)?;
    let mut pcm = Vec::with_capacity(n);
    let mut phases = [0.0f32; 3];
    let mut rng = Rng { state: 0xA5A5_1234 };
    let mut buf = [silence(0); 2];
    for ch in text.chars() {
        let k = phones_for(ch, &mut buf);
        for phone in buf.iter().take(k) {
            render_phone(phone, &mut pcm, &mut phases, &mut rng);
        }
    }
    debug_assert_eq!(pcm.len(), n);
    Ok(write_wav(&pcm, PCM_RATE))
}

/// RMS of `pcm` on the −1..1 scale.
pub fn rms(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let mut acc = 0.0f64;
    for s in pcm {
        let x = f64::from(*s) / 32_768.0;
        acc += x * x;
    }
    (acc / pcm.len() as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::parse_wav;

    #[test]
    fn hi_has_duration_and_energy_and_is_stable() {
        let wav = synthesize("HI").unwrap();
        let again = synthesize("HI").unwrap();
        assert_eq!(wav, again);
        let parsed = parse_wav(&wav).unwrap();
        assert_eq!(parsed.sample_rate, PCM_RATE);
        assert_eq!(parsed.pcm.len(), speech_sample_count("HI").unwrap());
        let dur = parsed.pcm.len() as f32 / PCM_RATE as f32;
        assert!((0.25..0.45).contains(&dur), "dur {dur}");
        let energy = rms(&parsed.pcm);
        assert!(energy > 0.05, "rms {energy}");
        let quiet = parse_wav(&synthesize("   ").unwrap()).unwrap();
        assert!(rms(&quiet.pcm) < energy * 0.05);
        assert_eq!(quiet.pcm.len(), speech_sample_count("   ").unwrap());
    }

    #[test]
    fn rejects_empty_and_non_ascii() {
        assert_eq!(synthesize(""), Err(SynthError::Empty));
        assert_eq!(synthesize("café"), Err(SynthError::NonAscii));
    }
}
