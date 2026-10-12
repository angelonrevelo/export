use std::fmt;

/// Recoverable decode, encode, or budget failure.
///
/// Library paths return this instead of panicking. A bad file is a counted
/// failure for the caller, not a process crash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    Truncated,
    Unsupported,
    Invalid,
    TooLarge,
    OverBudget,
}

impl Error {
    pub const fn new(code: ErrorCode) -> Self {
        Self { code }
    }
}

pub const TRUNCATED: Error = Error::new(ErrorCode::Truncated);
pub const UNSUPPORTED: Error = Error::new(ErrorCode::Unsupported);
pub const INVALID: Error = Error::new(ErrorCode::Invalid);
pub const TOO_LARGE: Error = Error::new(ErrorCode::TooLarge);
pub const OVER_BUDGET: Error = Error::new(ErrorCode::OverBudget);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.code {
            ErrorCode::Truncated => "truncated image",
            ErrorCode::Unsupported => "unsupported image",
            ErrorCode::Invalid => "invalid image",
            ErrorCode::TooLarge => "image exceeds the pixel cap",
            ErrorCode::OverBudget => "image does not fit the byte cap",
        })
    }
}

impl std::error::Error for Error {}
