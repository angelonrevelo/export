use std::fmt;

/// A refused encode. Library paths return this instead of panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `encode` was asked for a P-frame. A P-frame is written only by [`crate::encode_ip`], after the I-frame.
    PredictedFrame,
    /// A B-frame was requested.
    BipredictedFrame,
    /// A second pass was requested. There is one pass.
    SecondPass,
    /// Any thread count other than 1. Threading is off.
    Threading,
    /// QP was outside 0..=51. One QP is used for the whole frame.
    Qp,
    /// Width or height was not a multiple of 16.
    Size,
    /// Y, U, or V did not match 4:2:0.
    Plane,
    /// Frame is above the cap this encoder will allocate.
    TooLarge,
    /// The P-frame dimensions do not match the I-frame.
    Shape,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::PredictedFrame => "a P-frame is encoded only after the I-frame",
            Error::BipredictedFrame => "B-frames are not encoded",
            Error::SecondPass => "a second pass is not encoded",
            Error::Threading => "threading is off",
            Error::Qp => "qp must be a single value from 0 to 51",
            Error::Size => "width and height must be multiples of 16",
            Error::Plane => "planes are not 4:2:0",
            Error::TooLarge => "frame exceeds 128 by 128",
            Error::Shape => "the P-frame size does not match the I-frame",
        })
    }
}

impl std::error::Error for Error {}
