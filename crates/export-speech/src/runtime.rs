//! Integer kernels for a tiny acoustic step: matmul, conv1d, softmax, layer norm.
//!
//! Values are plain `i32`. Products accumulate in `i64` and the call fails with
//! [`RuntimeError::Overflow`] rather than wrapping. Softmax emits parts-per-million
//! using a 17-entry `exp` table (see [`EXP_MILLI`]). None of these functions allocate.

/// What an integer kernel refused to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    /// A slice length does not match `m`, `k`, `n`, or the convolution shape.
    Shape,
    /// An accumulator does not fit in `i32`.
    Overflow,
    /// A dimension or a kernel was zero.
    Empty,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Shape => write!(f, "shape mismatch"),
            RuntimeError::Overflow => write!(f, "i32 overflow"),
            RuntimeError::Empty => write!(f, "empty dimension"),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// `round(1000 * exp(-d))` for `d = 0..=16`. Past 16 the table is treated as 0.
/// `exp(-1) = 0.367879… → 368`, `exp(-2) = 0.135335… → 135`, `exp(-3) = 0.049787… → 50`.
pub const EXP_MILLI: [i32; 17] = [1000, 368, 135, 50, 18, 7, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// `c = a · b` for row-major `a` (`m×k`) and `b` (`k×n`). `c` is `m×n`.
///
/// The loop order is `i, t, j` so `b` and `c` are walked in row order.
/// The caller owns `c`. This function does not allocate.
pub fn matmul_i32(
    a: &[i32],
    b: &[i32],
    c: &mut [i32],
    m: usize,
    k: usize,
    n: usize,
) -> Result<(), RuntimeError> {
    if m == 0 || k == 0 || n == 0 {
        return Err(RuntimeError::Empty);
    }
    if a.len() != m * k || b.len() != k * n || c.len() != m * n {
        return Err(RuntimeError::Shape);
    }
    for i in 0..m {
        for j in 0..n {
            c[i * n + j] = 0;
        }
        for t in 0..k {
            let av = a[i * k + t];
            let row = t * n;
            for j in 0..n {
                let prod = i64::from(av) * i64::from(b[row + j]);
                let acc = i64::from(c[i * n + j]) + prod;
                c[i * n + j] = i32::try_from(acc).map_err(|_| RuntimeError::Overflow)?;
            }
        }
    }
    Ok(())
}

/// Valid (no padding) 1-D convolution. `out[(i)] = Σ_t input[i*stride+t] * kernel[t]`.
pub fn conv1d_i32(
    input: &[i32],
    kernel: &[i32],
    out: &mut [i32],
    stride: usize,
) -> Result<(), RuntimeError> {
    if kernel.is_empty() || stride == 0 || input.len() < kernel.len() {
        return Err(RuntimeError::Empty);
    }
    let out_len = (input.len() - kernel.len()) / stride + 1;
    if out.len() != out_len {
        return Err(RuntimeError::Shape);
    }
    for (i, slot) in out.iter_mut().enumerate() {
        let base = i * stride;
        let mut acc: i64 = 0;
        for (t, &kv) in kernel.iter().enumerate() {
            acc += i64::from(input[base + t]) * i64::from(kv);
        }
        *slot = i32::try_from(acc).map_err(|_| RuntimeError::Overflow)?;
    }
    Ok(())
}

/// Integer square root, floored. `isqrt(0) = 0`.
pub fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Population layer norm with truncating integer division.
///
/// `mean = Σx / n`, `var = Σ(x-mean)² / n`, `std = isqrt(var)`.
/// `y = (x-mean) * gamma / std + beta`. When `std == 0`, `y = beta`.
pub fn layer_norm_i32(
    x: &[i32],
    gamma: &[i32],
    beta: &[i32],
    out: &mut [i32],
) -> Result<(), RuntimeError> {
    let n = x.len();
    if n == 0 {
        return Err(RuntimeError::Empty);
    }
    if gamma.len() != n || beta.len() != n || out.len() != n {
        return Err(RuntimeError::Shape);
    }
    let mut sum: i64 = 0;
    for &v in x {
        sum += i64::from(v);
    }
    let mean = sum / n as i64;
    let mut var_sum: i64 = 0;
    for &v in x {
        let d = i64::from(v) - mean;
        var_sum += d * d;
    }
    let var = var_sum / n as i64;
    if var < 0 {
        return Err(RuntimeError::Overflow);
    }
    let std_u = isqrt(var as u64);
    if std_u > i32::MAX as u64 {
        return Err(RuntimeError::Overflow);
    }
    let stdv = std_u as i32;
    if stdv == 0 {
        out.copy_from_slice(beta);
        return Ok(());
    }
    let mean_i = i32::try_from(mean).map_err(|_| RuntimeError::Overflow)?;
    for i in 0..n {
        let centered = i64::from(x[i]) - i64::from(mean_i);
        let scaled = centered * i64::from(gamma[i]) / i64::from(stdv);
        let y = scaled + i64::from(beta[i]);
        out[i] = i32::try_from(y).map_err(|_| RuntimeError::Overflow)?;
    }
    Ok(())
}

/// Softmax into parts per million (`Σ out ≈ 1_000_000`, truncation remainder dropped).
///
/// Subtracts the max logit, looks up [`EXP_MILLI`] at the integer distance, then
/// `out[i] = exp_i * 1_000_000 / Σexp`. Ties in the later argmax stay at the
/// lower index because this function does not break ties; it only normalizes.
pub fn softmax_ppm(logits: &[i32], out: &mut [i32]) -> Result<(), RuntimeError> {
    if logits.is_empty() {
        return Err(RuntimeError::Empty);
    }
    if out.len() != logits.len() {
        return Err(RuntimeError::Shape);
    }
    let mut max = logits[0];
    for &v in &logits[1..] {
        if v > max {
            max = v;
        }
    }
    let mut sum: i64 = 0;
    for (logit, slot) in logits.iter().zip(out.iter_mut()) {
        let d = i64::from(*logit) - i64::from(max);
        let e = if d > 0 {
            return Err(RuntimeError::Overflow);
        } else if d < -16 {
            0
        } else {
            EXP_MILLI[(-d) as usize]
        };
        *slot = e;
        sum += i64::from(e);
    }
    if sum == 0 {
        return Err(RuntimeError::Empty);
    }
    for slot in out.iter_mut() {
        let ppm = i64::from(*slot) * 1_000_000 / sum;
        *slot = i32::try_from(ppm).map_err(|_| RuntimeError::Overflow)?;
    }
    Ok(())
}

/// Lowest index of the maximum. Ties stay at the smaller index.
pub fn argmax(row: &[i32]) -> Option<usize> {
    let mut best_i = 0usize;
    let mut best_v = *row.first()?;
    for (i, &v) in row.iter().enumerate().skip(1) {
        if v > best_v {
            best_v = v;
            best_i = i;
        }
    }
    Some(best_i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matmul_2x2_hand() {
        // [[1, 2], [3, 4]] · [[5, 6], [7, 8]] = [[19, 22], [43, 50]]
        let a = [1, 2, 3, 4];
        let b = [5, 6, 7, 8];
        let mut c = [0; 4];
        matmul_i32(&a, &b, &mut c, 2, 2, 2).unwrap();
        assert_eq!(c, [19, 22, 43, 50]);
    }

    #[test]
    fn matmul_2x3_hand() {
        // [1 2 3; 4 5 6] · [7 8; 9 10; 11 12] = [58 64; 139 154]
        let a = [1, 2, 3, 4, 5, 6];
        let b = [7, 8, 9, 10, 11, 12];
        let mut c = [0; 4];
        matmul_i32(&a, &b, &mut c, 2, 3, 2).unwrap();
        assert_eq!(c, [58, 64, 139, 154]);
    }

    #[test]
    fn matmul_overflow_is_an_error() {
        let a = [i32::MAX];
        let b = [3];
        let mut c = [0];
        assert_eq!(
            matmul_i32(&a, &b, &mut c, 1, 1, 1),
            Err(RuntimeError::Overflow)
        );
    }

    #[test]
    fn conv1d_hand() {
        // kernel [1, 0, -1] is a first difference across a gap.
        // input [1, 2, 3, 4, 5]
        // stride 1 → [-2, -2, -2]
        let input = [1, 2, 3, 4, 5];
        let kernel = [1, 0, -1];
        let mut out = [0; 3];
        conv1d_i32(&input, &kernel, &mut out, 1).unwrap();
        assert_eq!(out, [-2, -2, -2]);
        let mut hopped = [0; 2];
        conv1d_i32(&input, &kernel, &mut hopped, 2).unwrap();
        assert_eq!(hopped, [-2, -2]);
    }

    #[test]
    fn isqrt_hand() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(15), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(17), 4);
        assert_eq!(isqrt(3), 1);
    }

    #[test]
    fn layer_norm_hand() {
        // x = [1, 2, 3, 6], mean = 3, var = 14/4 = 3, std = isqrt(3) = 1
        // gamma = 2, beta = 10
        // y = [-2*2+10, -1*2+10, 0+10, 3*2+10] = [6, 8, 10, 16]
        let x = [1, 2, 3, 6];
        let gamma = [2, 2, 2, 2];
        let beta = [10, 10, 10, 10];
        let mut y = [0; 4];
        layer_norm_i32(&x, &gamma, &beta, &mut y).unwrap();
        assert_eq!(y, [6, 8, 10, 16]);
    }

    #[test]
    fn layer_norm_zero_std_is_beta() {
        let x = [5, 5, 5];
        let gamma = [9, 9, 9];
        let beta = [1, 2, 3];
        let mut y = [0; 3];
        layer_norm_i32(&x, &gamma, &beta, &mut y).unwrap();
        assert_eq!(y, [1, 2, 3]);
    }

    #[test]
    fn softmax_hand_ppm() {
        // logits [2, 1, 0] → shifts [0, -1, -2]
        // exp milli [1000, 368, 135], sum 1503
        // 1000 * 1_000_000 / 1503 = 665335
        // 368  * 1_000_000 / 1503 = 244843
        // 135  * 1_000_000 / 1503 = 89820
        // remainder 2 ppm is dropped (sum 999998)
        let logits = [2, 1, 0];
        let mut out = [0; 3];
        softmax_ppm(&logits, &mut out).unwrap();
        assert_eq!(out, [665_335, 244_843, 89_820]);
        assert_eq!(out.iter().sum::<i32>(), 999_998);
    }

    #[test]
    fn argmax_tie_keeps_low_index() {
        assert_eq!(argmax(&[1, 1, 0]), Some(0));
        assert_eq!(argmax(&[0, 3, 3]), Some(1));
        assert_eq!(argmax(&[]), None);
    }
}
