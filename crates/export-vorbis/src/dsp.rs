//! Short-block IMDCT and the overlap ffmpeg uses for one 64-sample frame.

pub(crate) const BINS: usize = 32;

pub(crate) fn unpack_vorbis_float(val: u32) -> f32 {
    let mut mant = (val & 0x001f_ffff) as i32;
    let exp = ((val & 0x7fe0_0000) >> 21) as i32;
    if val & 0x8000_0000 != 0 {
        mant = -mant;
    }
    (mant as f64 * 2f64.powi(exp - 20 - 768)) as f32
}

pub(crate) fn pack_vorbis_float(val: f32) -> u32 {
    if val == 0.0 {
        return 0;
    }
    let mut sign = 0u32;
    let mut x = f64::from(val);
    if x < 0.0 {
        sign = 0x8000_0000;
        x = -x;
    }
    let mut exp = x.log2().floor() as i32 + 768;
    let mut mant = (x * 2f64.powi(788 - exp)).round() as i32;
    if mant >= 0x20_0000 {
        mant >>= 1;
        exp += 1;
    }
    if exp < 0 || mant <= 0 {
        return sign;
    }
    if exp > 1023 {
        exp = 1023;
    }
    sign | ((exp as u32) << 21) | (mant as u32 & 0x001f_ffff)
}

/// Inverse MDCT matching ffmpeg's naive inverse at scale -1 for a 32-bin block.
pub(crate) fn imdct(spectrum: &[f64; BINS]) -> [f64; BINS] {
    let scale = -1.0;
    let length = BINS / 2;
    let len2 = length * 2;
    let phase = std::f64::consts::PI / (4.0 * len2 as f64);
    let mut time = [0.0; BINS];
    for i in 0..length {
        let mut sum_d = 0.0;
        let mut sum_u = 0.0;
        let i_d = phase * (4 * length - 2 * i - 1) as f64;
        let i_u = phase * (3 * len2 + 2 * i + 1) as f64;
        for (j, coeff) in spectrum.iter().enumerate() {
            let angle = (2 * j + 1) as f64;
            sum_d += (angle * i_d).cos() * coeff;
            sum_u += (angle * i_u).cos() * coeff;
        }
        time[i] = sum_d * scale;
        time[i + length] = -sum_u * scale;
    }
    time
}

pub(crate) fn mdct(time: &[f64; BINS]) -> [f64; BINS] {
    let mut matrix = [[0.0; BINS]; BINS];
    for bin in 0..BINS {
        let mut impulse = [0.0; BINS];
        impulse[bin] = 1.0;
        let column = imdct(&impulse);
        for row in 0..BINS {
            matrix[row][bin] = column[row];
        }
    }
    solve(&matrix, time)
}

fn solve(matrix: &[[f64; BINS]; BINS], time: &[f64; BINS]) -> [f64; BINS] {
    let mut rows = [[0.0; BINS + 1]; BINS];
    for row in 0..BINS {
        rows[row][..BINS].copy_from_slice(&matrix[row]);
        rows[row][BINS] = time[row];
    }
    for col in 0..BINS {
        let mut pivot = col;
        for row in col + 1..BINS {
            if rows[row][col].abs() > rows[pivot][col].abs() {
                pivot = row;
            }
        }
        rows.swap(col, pivot);
        let div = rows[col][col];
        for value in &mut rows[col][col..] {
            *value /= div;
        }
        for row in 0..BINS {
            if row == col {
                continue;
            }
            let factor = rows[row][col];
            for next in col..=BINS {
                rows[row][next] -= factor * rows[col][next];
            }
        }
    }
    let mut spectrum = [0.0; BINS];
    for row in 0..BINS {
        spectrum[row] = rows[row][BINS];
    }
    spectrum
}

/// Delay frame and tone frame whose overlap is `sine`.
pub(crate) fn overlap_pair(sine: &[f64; BINS]) -> ([f64; BINS], [f64; BINS]) {
    let window = short_window();
    let mut saved = [0.0; 16];
    let mut buf = [0.0; 16];
    for j in 0..16 {
        let a = window[15 - j];
        let c = window[16 + j];
        let lo = sine[15 - j];
        let hi = sine[16 + j];
        let den = a * a + c * c;
        saved[15 - j] = (c * lo + a * hi) / den;
        buf[j] = (c * hi - a * lo) / den;
    }
    let mut delay = [0.0; BINS];
    delay[16..].copy_from_slice(&saved);
    let mut tone = [0.0; BINS];
    tone[..16].copy_from_slice(&buf);
    // The right quarter is the overlap state this frame needs, so the same
    // frame can follow itself and still reconstruct `sine`.
    tone[16..].copy_from_slice(&saved);
    (delay, tone)
}

pub(crate) fn short_window() -> [f64; BINS] {
    let mut window = [0.0; BINS];
    let block = (BINS * 2) as f64;
    for (i, sample) in window.iter_mut().enumerate() {
        let z = (i as f64 + 0.5) / block * std::f64::consts::PI;
        *sample = (0.5 * std::f64::consts::PI * z.sin() * z.sin()).sin();
    }
    window
}

/// Overlap-add one short frame onto the previous right quarter.
/// Returns PCM and the quarter to keep for the next frame.
pub(crate) fn overlap(previous: &[f64; 16], time: &[f64; BINS]) -> ([f64; BINS], [f64; 16]) {
    let window = short_window();
    let mut pcm = [0.0; BINS];
    for k in 0..16 {
        let j = 15 - k;
        let saved = previous[k];
        let sample = time[j];
        let wi = window[k];
        let wj = window[31 - k];
        pcm[k] = saved * wj - sample * wi;
        pcm[16 + j] = saved * wi + sample * wj;
    }
    let mut next = [0.0; 16];
    next.copy_from_slice(&time[16..32]);
    (pcm, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half_scale_sine() -> [f64; BINS] {
        let mut sine = [0.0; BINS];
        for (n, sample) in sine.iter_mut().enumerate() {
            let phase = 2.0 * std::f64::consts::PI * 440.0 * n as f64 / 48_000.0;
            *sample = 0.5 * phase.sin();
        }
        sine
    }

    #[test]
    fn window_matches_short_block() {
        let window = short_window();
        assert!((window[0] - 0.0009460463).abs() < 1.0e-8);
    }

    #[test]
    fn overlap_reconstructs_sine() {
        let sine = half_scale_sine();
        let (delay, tone) = overlap_pair(&sine);
        for spectrum in [mdct(&delay), mdct(&tone)] {
            let peak = spectrum
                .iter()
                .fold(0.0f64, |peak, value| peak.max(value.abs()));
            assert!(peak < 0.5, "coefficient peak {peak}");
        }
        let delay_time = imdct(&mdct(&delay));
        let tone_time = imdct(&mdct(&tone));
        let (_first, saved) = overlap(&[0.0; 16], &delay_time);
        let (pcm, next) = overlap(&saved, &tone_time);
        for (got, want) in pcm.iter().zip(sine.iter()) {
            assert!((got - want).abs() < 1.0e-9);
        }
        let (second, _) = overlap(&next, &tone_time);
        for (got, want) in second.iter().zip(sine.iter()) {
            assert!((got - want).abs() < 1.0e-9);
        }
    }

    #[test]
    fn continuation_fits_grid() {
        let mut target = [0.0; BINS];
        for (offset, sample) in target.iter_mut().enumerate() {
            let n = (32 + offset) as f64;
            let phase = 2.0 * std::f64::consts::PI * 440.0 * n / 48_000.0;
            *sample = 0.5 * phase.sin();
        }
        let (bridge, tone) = overlap_pair(&target);
        let mut left = tone;
        left[16..].fill(0.0);
        for (name, time) in [("bridge", bridge), ("tone", tone), ("left", left)] {
            let peak = mdct(&time)
                .iter()
                .fold(0.0f64, |peak, value| peak.max(value.abs()));
            assert!(peak < 0.5, "{name} peak {peak}");
        }
    }
}
