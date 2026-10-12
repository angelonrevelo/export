use crate::sniff::MediaKind;
use std::fmt;

/// Recoverable media failure. Library paths return this instead of panicking.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Truncated {
        at: usize,
        need: usize,
    },
    Corrupt(&'static str),
    Unsupported(&'static str),
    /// Sniff succeeded but the job does not handle this container or codec.
    Kind {
        found: MediaKind,
        job: &'static str,
    },
    /// Reader exceeded the memory cap before a codec ran.
    Limit {
        cap: u64,
        got: u64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated { at, need } => {
                write!(f, "truncated at {at}, need {need} more bytes")
            }
            Error::Corrupt(msg) => write!(f, "corrupt: {msg}"),
            Error::Unsupported(msg) => write!(f, "unsupported: {msg}"),
            Error::Kind { found, job } => write!(f, "{job} does not accept {found}"),
            Error::Limit { cap, got } => write!(f, "input {got} bytes exceeds cap {cap}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
