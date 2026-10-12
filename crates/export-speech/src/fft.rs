//! In-place radix-2 complex FFT. No allocation.
//!
//! Forward transform uses the engineering sign (e^{-j 2π}). [`ifft`] undoes it.
//! Length must be a power of two and at least 2. The hot path never allocates;
//! the caller owns both planes.

use std::f32::consts::TAU;

/// Forward FFT. Returns false when the planes differ in length or the length
/// is not a power of two ≥ 2. On failure the buffers are left untouched.
pub fn fft(re: &mut [f32], im: &mut [f32]) -> bool {
    let n = re.len();
    if n != im.len() || n < 2 || !n.is_power_of_two() {
        return false;
    }
    bit_reverse(re, im);
    let mut len = 2;
    while len <= n {
        let ang = -TAU / len as f32;
        let wlen_re = ang.cos();
        let wlen_im = ang.sin();
        let mut i = 0;
        while i < n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..len / 2 {
                let a = i + k;
                let b = a + len / 2;
                let v_re = re[b] * w_re - im[b] * w_im;
                let v_im = re[b] * w_im + im[b] * w_re;
                re[b] = re[a] - v_re;
                im[b] = im[a] - v_im;
                re[a] += v_re;
                im[a] += v_im;
                let next_re = w_re * wlen_re - w_im * wlen_im;
                w_im = w_re * wlen_im + w_im * wlen_re;
                w_re = next_re;
            }
            i += len;
        }
        len <<= 1;
    }
    true
}

/// Inverse of [`fft`]. Divides by `n`. Same length rules.
pub fn ifft(re: &mut [f32], im: &mut [f32]) -> bool {
    if re.len() != im.len() || re.len() < 2 || !re.len().is_power_of_two() {
        return false;
    }
    for v in im.iter_mut() {
        *v = -*v;
    }
    if !fft(re, im) {
        return false;
    }
    let n = re.len() as f32;
    for (re_i, im_i) in re.iter_mut().zip(im.iter_mut()) {
        *re_i /= n;
        *im_i = -*im_i / n;
    }
    true
}

fn bit_reverse(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn impulse_is_flat() {
        let mut re = [0.0f32; 8];
        let mut im = [0.0f32; 8];
        re[0] = 1.0;
        assert!(fft(&mut re, &mut im));
        for k in 0..8 {
            assert!(close(re[k], 1.0), "re[{k}]={}", re[k]);
            assert!(close(im[k], 0.0), "im[{k}]={}", im[k]);
        }
    }

    #[test]
    fn cosine_peaks_on_its_bin() {
        let n = 8;
        let mut re = [0.0f32; 8];
        let mut im = [0.0f32; 8];
        for (i, sample) in re.iter_mut().enumerate() {
            *sample = (TAU * i as f32 / n as f32).cos();
        }
        assert!(fft(&mut re, &mut im));
        let power = |k: usize| re[k] * re[k] + im[k] * im[k];
        let p1 = power(1);
        for k in 0..n {
            if k != 1 && k != n - 1 {
                assert!(power(k) < p1 * 0.01, "bin {k} power {}", power(k));
            }
        }
    }

    #[test]
    fn roundtrip_hand_vector() {
        let original = [1.0f32, -2.0, 0.5, 4.0, 0.0, 3.0, -1.0, 0.25];
        let mut re = original;
        let mut im = [0.0f32; 8];
        assert!(fft(&mut re, &mut im));
        assert!(ifft(&mut re, &mut im));
        for i in 0..8 {
            assert!(
                close(re[i], original[i]),
                "{i}: {} vs {}",
                re[i],
                original[i]
            );
            assert!(close(im[i], 0.0), "im {i} {}", im[i]);
        }
    }

    #[test]
    fn rejects_bad_length() {
        let mut re = [0.0f32; 6];
        let mut im = [0.0f32; 6];
        assert!(!fft(&mut re, &mut im));
        assert!(!ifft(&mut re, &mut im));
    }
}
