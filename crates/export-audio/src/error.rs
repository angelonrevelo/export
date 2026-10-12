use crate::sniff::Kind;
use std::fmt;

/// Recoverable audio failure. Library paths return this instead of panicking.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Truncated { at: usize, need: usize },
    Corrupt(&'static str),
    Unsupported(&'static str),
    /// Sniff succeeded but this job does not handle the container.
    Kind { found: Kind, job: &'static str },
    /// Input or decoded sample count exceeded the cap before a result was kept.
    Limit { cap: u64, got: u64 },
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
            Error::Limit { cap, got } => write!(f, "{got} exceeds cap {cap}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
