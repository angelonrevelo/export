//! Long-window AAC MDCT (2048 → 1024) and its spec inverse.
//!
//! Inverse matches ISO/IEC 14496-3: N = 2048, n0 = 512.5,
//! `x[n] = (2/N) * sum_k X[k] * cos(2π/N * (n + n0) * (k + 1/2))`.
//! The forward transform is the pairing sum (no extra scale) so that
//! sine-window TDAC reconstructs a block that has a neighbor.

const N: usize = 1024;
const WINDOW: usize = 2048;

fn window_at(n: usize) -> f64 {
    (std::f64::consts::PI / WINDOW as f64 * (n as f64 + 0.5)).sin()
}

fn mdct(x: &[f64; WINDOW]) -> [f64; N] {
    let mut y = [0.0; N];
    let pi_over_n = std::f64::consts::PI / N as f64;
    for (k, bin) in y.iter_mut().enumerate() {
        let freq = (k as f64 + 0.5) * pi_over_n;
        let (cs, sn) = (freq.cos(), freq.sin());
        let mut acc = 0.0;
        let mut n = 0;
        while n < WINDOW {
            let ang = (n as f64 + 512.5) * freq;
            let (mut c, mut s) = (ang.cos(), ang.sin());
            let end = (n + 32).min(WINDOW);
            while n < end {
                acc += x[n] * c;
                let nc = c * cs - s * sn;
                s = s * cs + c * sn;
                c = nc;
                n += 1;
            }
        }
        *bin = acc;
    }
    y
}

fn imdct(spec: &[f64; N]) -> [f64; WINDOW] {
    let mut y = [0.0; WINDOW];
    let pi_over_n = std::f64::consts::PI / N as f64;
    let scale = 1.0 / N as f64;
    for (n, sample) in y.iter_mut().enumerate() {
        let time = n as f64 + 512.5;
        let step = time * pi_over_n;
        let (cs, sn) = (step.cos(), step.sin());
        let mut acc = 0.0;
        let mut k = 0;
        while k < N {
            let ang = (k as f64 + 0.5) * step;
            let (mut c, mut s) = (ang.cos(), ang.sin());
            let end = (k + 32).min(N);
            while k < end {
                acc += spec[k] * c;
                let nc = c * cs - s * sn;
                s = s * cs + c * sn;
                c = nc;
                k += 1;
            }
        }
        *sample = acc * scale;
    }
    y
}

/// Window and forward MDCT of one 2048-sample block.
pub fn analysis(block: &[f64; WINDOW]) -> [f64; N] {
    let mut windowed = [0.0; WINDOW];
    for n in 0..WINDOW {
        windowed[n] = block[n] * window_at(n);
    }
    let mut spec = mdct(&windowed);
    // Decoder IMDCT uses 2/N. The extra 2 here is the analysis gain that
    // makes sine-window overlap-add reconstruct the PCM.
    for bin in &mut spec {
        *bin *= 2.0;
    }
    spec
}

/// IMDCT, sine window, overlap-add with a zero previous frame.
/// Returns the 1024 samples a decoder emits for a lone frame.
pub fn synthesis_one_frame(spec: &[f64; N]) -> [f64; N] {
    let im = imdct(spec);
    let mut out = [0.0; N];
    for n in 0..N {
        out[n] = window_at(n) * im[n];
    }
    out
}

/// IMDCT, window, and overlap-add. `overlap` is the previous frame's tail.
#[cfg(test)]
pub fn synthesis_overlap(spec: &[f64; N], overlap: &mut [f64; N]) -> [f64; N] {
    let im = imdct(spec);
    let mut out = [0.0; N];
    for n in 0..N {
        let z = window_at(n) * im[n];
        out[n] = z + overlap[n];
        overlap[n] = window_at(n + N) * im[n + N];
    }
    out
}

#[cfg(test)]
fn naive_bin(x: &[f64; WINDOW], k: usize) -> f64 {
    let freq = (k as f64 + 0.5) * (std::f64::consts::PI / N as f64);
    let mut acc = 0.0;
    for (n, sample) in x.iter().enumerate() {
        acc += sample * ((n as f64 + 512.5) * freq).cos();
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_mdct_matches_naive_bins() {
        let mut x = [0.0; WINDOW];
        for (i, sample) in x.iter_mut().enumerate() {
            *sample = ((i * 17) % 100) as f64 / 50.0 - 1.0;
        }
        let fast = mdct(&x);
        for k in [0usize, 1, 7, 50, 511, 1023] {
            let naive = naive_bin(&x, k);
            let err = (fast[k] - naive).abs();
            assert!(err < 1e-6, "bin {k} err {err}");
        }
    }

    #[test]
    fn two_frame_tdac_reconstructs_middle() {
        const FRAMES: usize = 4;
        let mut pcm = vec![0.0; FRAMES * N];
        for (i, sample) in pcm.iter_mut().enumerate() {
            let t = i as f64 / 44_100.0;
            *sample = (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 0.5;
        }
        let mut overlap = [0.0; N];
        let mut out = vec![0.0; FRAMES * N];
        for frame in 0..FRAMES {
            let mut block = [0.0; WINDOW];
            if frame == 0 {
                block[N..].copy_from_slice(&pcm[..N]);
            } else {
                let start = (frame - 1) * N;
                block.copy_from_slice(&pcm[start..start + WINDOW]);
            }
            let spec = analysis(&block);
            let hop = synthesis_overlap(&spec, &mut overlap);
            if frame > 0 {
                out[(frame - 1) * N..frame * N].copy_from_slice(&hop);
            }
        }
        // One-frame delay: the second block's overlap-add is input frame 0.
        let mut max_err = 0.0f64;
        for i in 0..N {
            max_err = max_err.max((out[i] - pcm[i]).abs());
            max_err = max_err.max((out[N + i] - pcm[N + i]).abs());
        }
        assert!(
            max_err < 1e-6,
            "unquantized TDAC max abs error {max_err}"
        );
    }
}
