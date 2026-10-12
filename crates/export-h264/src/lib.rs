//! Baseline H.264: one I-frame, then optionally one P-frame.
//!
//! The I-frame bitstream is Annex B: access unit delimiter, SPS, PPS, and one
//! IDR slice. Profile is Constrained Baseline, chroma is 4:2:0, there is one
//! slice, and one QP covers each frame. I-frame prediction is intra DC.
//! The P-frame is one slice with motion zero and that I-frame as its only
//! reference. Luma residual is a 4x4 transform. Chroma residual is the DC
//! coefficient only.
//!
//! A lone P-frame, a B-frame, a second pass, and any thread count other than 1
//! return an error. Nothing here links or spawns ffmpeg. This is not a
//! video shrink of a real MP4.

#![forbid(unsafe_code)]

mod bitstream;
mod cavlc;
mod encode;
mod error;

pub use error::Error;

/// What a one-frame [`encode`] call asked for. A P-frame is [`encode_ip`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Intra,
    Predicted,
    Bipredicted,
}

/// One pass. [`Pass::Second`] is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Single,
    Second,
}

/// Encode settings. Threading is off: `threads` must be 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeOptions {
    pub qp: u8,
    pub picture: Picture,
    pub pass: Pass,
    pub threads: u8,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            qp: FIXTURE_QP,
            picture: Picture::Intra,
            pass: Pass::Single,
            threads: 1,
        }
    }
}

/// QP used for the published 32×32 fixture.
pub const FIXTURE_QP: u8 = 26;

/// Planar 4:2:0, 8-bit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Yuv420 {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl Yuv420 {
    pub fn new(width: u32, height: u32, y: Vec<u8>, u: Vec<u8>, v: Vec<u8>) -> Result<Self, Error> {
        let frame = Self {
            width,
            height,
            y,
            u,
            v,
        };
        frame.check()?;
        Ok(frame)
    }

    pub fn flat(width: u32, height: u32, y: u8, u: u8, v: u8) -> Result<Self, Error> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err(Error::Size);
        }
        let cw = (width / 2) as usize;
        let ch = (height / 2) as usize;
        Ok(Self {
            width,
            height,
            y: vec![y; (width as usize) * (height as usize)],
            u: vec![u; cw * ch],
            v: vec![v; cw * ch],
        })
    }

    fn check(&self) -> Result<(), Error> {
        if self.width == 0 || self.height == 0 || self.width % 16 != 0 || self.height % 16 != 0 {
            return Err(Error::Size);
        }
        if self.width > 128 || self.height > 128 {
            return Err(Error::TooLarge);
        }
        let y = (self.width as usize) * (self.height as usize);
        let c = y / 4;
        if self.y.len() != y || self.u.len() != c || self.v.len() != c {
            return Err(Error::Plane);
        }
        Ok(())
    }

    /// Tightly packed I420.
    pub fn i420(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.y.len() + self.u.len() + self.v.len());
        out.extend_from_slice(&self.y);
        out.extend_from_slice(&self.u);
        out.extend_from_slice(&self.v);
        out
    }
}

/// Annex B bytes and the encoder's reconstruction of that one frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoded {
    pub annex_b: Vec<u8>,
    pub reconstructed: Yuv420,
    pub qp: u8,
}

/// Annex B for the I-frame followed by one P-frame, plus both reconstructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedPair {
    pub annex_b: Vec<u8>,
    pub intra: Yuv420,
    pub predicted: Yuv420,
    pub qp: u8,
}

/// Encode one I-frame. A lone P-frame, a B-frame, a second pass, and threads are refused.
pub fn encode(frame: &Yuv420, opts: &EncodeOptions) -> Result<Encoded, Error> {
    frame.check()?;
    if opts.qp > 51 {
        return Err(Error::Qp);
    }
    match opts.picture {
        Picture::Intra => {}
        Picture::Predicted => return Err(Error::PredictedFrame),
        Picture::Bipredicted => return Err(Error::BipredictedFrame),
    }
    match opts.pass {
        Pass::Single => {}
        Pass::Second => return Err(Error::SecondPass),
    }
    if opts.threads != 1 {
        return Err(Error::Threading);
    }
    let (annex_b, reconstructed) = encode::encode_iframe(frame, opts.qp)?;
    Ok(Encoded {
        annex_b,
        reconstructed,
        qp: opts.qp,
    })
}

fn check_common(opts: &EncodeOptions) -> Result<(), Error> {
    if opts.qp > 51 {
        return Err(Error::Qp);
    }
    match opts.pass {
        Pass::Single => {}
        Pass::Second => return Err(Error::SecondPass),
    }
    if opts.threads != 1 {
        return Err(Error::Threading);
    }
    Ok(())
}

/// I-frame, then one P-frame that references only that reconstruction.
///
/// The leading bytes are exactly [`encode`] of `intra`. B-frames and a second
/// pass are refused.
pub fn encode_ip(intra: &Yuv420, predicted: &Yuv420, opts: &EncodeOptions) -> Result<EncodedPair, Error> {
    intra.check()?;
    predicted.check()?;
    check_common(opts)?;
    if matches!(opts.picture, Picture::Bipredicted) {
        return Err(Error::BipredictedFrame);
    }
    if intra.width != predicted.width || intra.height != predicted.height {
        return Err(Error::Shape);
    }
    let (i_bytes, i_rec) = encode::encode_iframe(intra, opts.qp)?;
    let (p_bytes, p_rec) = encode::encode_pframe(predicted, &i_rec, opts.qp)?;
    let mut annex_b = i_bytes;
    annex_b.extend(p_bytes);
    Ok(EncodedPair {
        annex_b,
        intra: i_rec,
        predicted: p_rec,
        qp: opts.qp,
    })
}

/// Deterministic 32×32 4:2:0 pattern used for the published measurement.
pub fn fixture_32x32() -> Yuv420 {
    let mut y = vec![0u8; 32 * 32];
    let mut u = vec![0u8; 16 * 16];
    let mut v = vec![0u8; 16 * 16];
    for row in 0..32 {
        for col in 0..32 {
            y[row * 32 + col] = ((col * 7 + row * 3) % 180) as u8 + 40;
        }
    }
    for row in 0..16 {
        for col in 0..16 {
            u[row * 16 + col] = (100 + col * 3) as u8;
            v[row * 16 + col] = (120 + row * 4) as u8;
        }
    }
    Yuv420 {
        width: 32,
        height: 32,
        y,
        u,
        v,
    }
}

/// Second 32×32 frame. It is the I-frame fixture shifted a little, so the
/// P-frame residual is not zero.
pub fn fixture_32x32_predicted() -> Yuv420 {
    let mut frame = fixture_32x32();
    for sample in &mut frame.y {
        *sample = sample.saturating_add(16);
    }
    for sample in &mut frame.u {
        *sample = sample.saturating_add(4);
    }
    for sample in &mut frame.v {
        *sample = sample.saturating_sub(4);
    }
    frame
}

/// Mean absolute error over every Y, U, and V sample.
pub fn mean_absolute_error(a: &Yuv420, b: &Yuv420) -> f64 {
    let mut sum = 0u64;
    let mut n = 0u64;
    for (p, q) in a.y.iter().zip(b.y.iter()) {
        sum += (*p as i32 - *q as i32).unsigned_abs() as u64;
        n += 1;
    }
    for (p, q) in a.u.iter().zip(b.u.iter()) {
        sum += (*p as i32 - *q as i32).unsigned_abs() as u64;
        n += 1;
    }
    for (p, q) in a.v.iter().zip(b.v.iter()) {
        sum += (*p as i32 - *q as i32).unsigned_abs() as u64;
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}
