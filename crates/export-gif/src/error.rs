use std::fmt;

/// Recoverable decode, encode, or refusal.
///
/// Library paths return this. A bad file is not a panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
    message: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    Truncated,
    Unsupported,
    Invalid,
    BadLzw,
    TooLarge,
}

impl Error {
    pub const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    pub const fn message(self) -> &'static str {
        self.message
    }
}

pub const TRUNCATED: Error = Error::new(ErrorCode::Truncated, "truncated gif");
pub const UNSUPPORTED_VIDEO: Error =
    Error::new(ErrorCode::Unsupported, "video to gif is unsupported");
pub const NOT_GIF: Error = Error::new(ErrorCode::Invalid, "not a gif");
pub const INVALID: Error = Error::new(ErrorCode::Invalid, "invalid gif");
pub const BAD_LZW: Error = Error::new(ErrorCode::BadLzw, "bad lzw code");
pub const TOO_LARGE: Error = Error::new(ErrorCode::TooLarge, "gif exceeds the pixel cap");

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for Error {}
