use std::fmt;

/// A refused box. Library paths return this instead of panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The Annex B buffer ended before a whole I-frame.
    Truncated,
    /// The buffer is not one I-frame access unit.
    Frame,
    /// Width or height cannot be stored as one picture.
    Size,
    /// Any thread count other than 1. Threading is off.
    Threading,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Truncated => "the Annex B buffer is truncated",
            Error::Frame => "the buffer is not one I-frame",
            Error::Size => "width and height are not a usable picture",
            Error::Threading => "threading is off",
        })
    }
}

impl std::error::Error for Error {}
