//! Baseline H.264: one I-frame, then optionally one P-frame.
//!
//! The I-frame bitstream is Annex B: access unit delimiter, SPS, PPS, and one
//! IDR slice. Profile is Constrained Baseline, chroma is 4:2:0, there is one
//! slice, and one QP covers each frame. I-frame prediction is intra DC.
//! [`encode_ip`] writes one P slice with motion zero and that I-frame as its
//! only reference. [`encode_ip_motion`] uses one shared 16×16 vector instead.
//! [`encode_sequence`] writes that I-frame plus up to fifteen P-frames. Each
//! P-frame references the previous reconstruction and uses either zero motion
//! or one shared vector.
//! Luma residual is a 4x4 transform. Chroma residual is the DC coefficient only.
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
        // 320×240 is 300 macroblocks, inside level 1.1. 32×32 and 160×160 stay inside the same check.
        if self.width > 320 || self.height > 240 {
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

/// One quarter-luma motion vector, shared by every macroblock.
///
/// Both components must be multiples of 8 so the chroma sample is integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    pub mvx: i32,
    pub mvy: i32,
}

/// Four luma samples to the right. Chroma follows by two samples.
pub const SHIFTED_MV: Motion = Motion { mvx: 16, mvy: 0 };

/// I-frame, then one P-frame that references only that reconstruction.
///
/// Motion is zero. The leading bytes are exactly [`encode`] of `intra`.
/// B-frames and a second pass are refused.
pub fn encode_ip(
    intra: &Yuv420,
    predicted: &Yuv420,
    opts: &EncodeOptions,
) -> Result<EncodedPair, Error> {
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

/// Same as [`encode_ip`], with one non-zero or zero motion vector on the P-frame.
///
/// A zero vector matches [`encode_ip`] byte for byte. A lone P-frame is still
/// refused by [`encode`]. A B-frame and a second pass are refused here too.
pub fn encode_ip_motion(
    intra: &Yuv420,
    predicted: &Yuv420,
    opts: &EncodeOptions,
    motion: Motion,
) -> Result<EncodedPair, Error> {
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
    let (p_bytes, p_rec) =
        encode::encode_pframe_mv(predicted, &i_rec, opts.qp, motion.mvx, motion.mvy)?;
    let mut annex_b = i_bytes;
    annex_b.extend(p_bytes);
    Ok(EncodedPair {
        annex_b,
        intra: i_rec,
        predicted: p_rec,
        qp: opts.qp,
    })
}

/// One P-frame in a sequence, with the motion shared by every macroblock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredictedFrame {
    pub picture: Yuv420,
    pub motion: Motion,
}

/// Annex B for an I-frame and the P-frames after it, in display order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedSequence {
    pub annex_b: Vec<u8>,
    pub reconstructed: Vec<Yuv420>,
    pub qp: u8,
}

/// I-frame, then one to fifteen P-frames.
///
/// Each P-frame is predicted from the previous picture's reconstruction.
/// The last P-frame is not itself a reference. A single P-frame with the same
/// motion matches [`encode_ip_motion`]. A B-frame and a second pass are refused.
pub fn encode_sequence(
    intra: &Yuv420,
    predicted: &[PredictedFrame],
    opts: &EncodeOptions,
) -> Result<EncodedSequence, Error> {
    intra.check()?;
    check_common(opts)?;
    if matches!(opts.picture, Picture::Bipredicted) {
        return Err(Error::BipredictedFrame);
    }
    if predicted.is_empty() || predicted.len() > 15 {
        return Err(Error::Count);
    }
    for frame in predicted {
        frame.picture.check()?;
        if frame.picture.width != intra.width || frame.picture.height != intra.height {
            return Err(Error::Shape);
        }
    }
    let (mut annex_b, mut reference) = encode::encode_iframe(intra, opts.qp)?;
    let mut reconstructed = Vec::with_capacity(predicted.len() + 1);
    reconstructed.push(reference.clone());
    for (i, frame) in predicted.iter().enumerate() {
        let frame_num = (i + 1) as u32;
        let is_reference = i + 1 < predicted.len();
        let (bytes, rec) = encode::encode_pframe_at(
            &frame.picture,
            &reference,
            opts.qp,
            frame.motion.mvx,
            frame.motion.mvy,
            frame_num,
            is_reference,
        )?;
        annex_b.extend(bytes);
        reference = rec.clone();
        reconstructed.push(rec);
    }
    Ok(EncodedSequence {
        annex_b,
        reconstructed,
        qp: opts.qp,
    })
}

/// Deterministic 32×32 4:2:0 pattern used for the published measurement.
pub fn fixture_32x32() -> Yuv420 {
    fixture_pattern(32, 32)
}

/// Deterministic 160×160 4:2:0 pattern. Same formula as [`fixture_32x32`].
pub fn fixture_160x160() -> Yuv420 {
    fixture_pattern(160, 160)
}

/// Deterministic 320×240 4:2:0 pattern. Same formula as [`fixture_32x32`].
pub fn fixture_320x240() -> Yuv420 {
    fixture_pattern(320, 240)
}

fn fixture_pattern(width: u32, height: u32) -> Yuv420 {
    let w = width as usize;
    let h = height as usize;
    let cw = w / 2;
    let ch = h / 2;
    let mut y = vec![0u8; w * h];
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    for row in 0..h {
        for col in 0..w {
            y[row * w + col] = ((col * 7 + row * 3) % 180) as u8 + 40;
        }
    }
    for row in 0..ch {
        for col in 0..cw {
            u[row * cw + col] = (100 + (col * 3) % 120) as u8;
            v[row * cw + col] = (120 + (row * 4) % 100) as u8;
        }
    }
    Yuv420 {
        width,
        height,
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

/// Second 160×160 frame. Brighter than the I-frame, motion zero, residual not zero.
pub fn fixture_160x160_predicted() -> Yuv420 {
    let mut frame = fixture_160x160();
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

/// Second 320×240 frame. Brighter than the I-frame, so the P-frame residual is not zero.
pub fn fixture_320x240_predicted() -> Yuv420 {
    let mut frame = fixture_320x240();
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

fn shift_plane(src: &[u8], width: usize, height: usize, dx: i32, dy: i32) -> Vec<u8> {
    let mut out = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let sx = (x as i32 + dx).clamp(0, width as i32 - 1) as usize;
            let sy = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
            out[y * width + x] = src[sy * width + sx];
        }
    }
    out
}

/// The 32×32 fixture moved four luma samples to the left.
///
/// [`SHIFTED_MV`] predicts this picture from the I-frame.
pub fn fixture_32x32_shifted() -> Yuv420 {
    shift_frame(&fixture_32x32(), 4)
}

/// The 160×160 fixture moved four luma samples to the left.
///
/// [`SHIFTED_MV`] predicts this picture from that I-frame.
pub fn fixture_160x160_shifted() -> Yuv420 {
    shift_frame(&fixture_160x160(), 4)
}

fn shift_frame(src: &Yuv420, dx_luma: i32) -> Yuv420 {
    Yuv420 {
        width: src.width,
        height: src.height,
        y: shift_plane(&src.y, src.width as usize, src.height as usize, dx_luma, 0),
        u: shift_plane(
            &src.u,
            (src.width / 2) as usize,
            (src.height / 2) as usize,
            dx_luma / 2,
            0,
        ),
        v: shift_plane(
            &src.v,
            (src.width / 2) as usize,
            (src.height / 2) as usize,
            dx_luma / 2,
            0,
        ),
    }
}

/// Eight 32×32 pictures: the I-frame fixture and seven P-frames.
///
/// The first, third, fifth, and seventh P-frames hold still. The others move
/// four luma samples left and use [`SHIFTED_MV`].
pub fn fixture_32x32_eight() -> (Yuv420, Vec<PredictedFrame>) {
    sequence_from(fixture_32x32(), 7)
}

/// Eight 160×160 pictures, with the same hold-and-shift pattern as [`fixture_32x32_eight`].
pub fn fixture_160x160_eight() -> (Yuv420, Vec<PredictedFrame>) {
    sequence_from(fixture_160x160(), 7)
}

/// Sixteen 160×160 pictures. Same hold-and-shift pattern, continued for fifteen P-frames.
pub fn fixture_160x160_sixteen() -> (Yuv420, Vec<PredictedFrame>) {
    sequence_from(fixture_160x160(), 15)
}

fn sequence_from(intra: Yuv420, pframes: usize) -> (Yuv420, Vec<PredictedFrame>) {
    let mut picture = intra.clone();
    let mut predicted = Vec::with_capacity(pframes);
    for index in 0..pframes {
        let motion = if index % 2 == 1 {
            picture = shift_frame(&picture, 4);
            SHIFTED_MV
        } else {
            Motion { mvx: 0, mvy: 0 }
        };
        predicted.push(PredictedFrame {
            picture: picture.clone(),
            motion,
        });
    }
    (intra, predicted)
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
