use std::fmt;

/// Recoverable archive failure. Truncation and bit flips return this.
/// They do not panic and they do not yield a successful entry.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The bytes are not a zip this reader can trust.
    Format(&'static str),
    /// CRC-32 of the entry payload does not match the central directory.
    ChecksumMismatch,
    /// A real zip feature this crate does not implement.
    Unsupported(&'static str),
    /// A size or count the on-disk format cannot record without zip64.
    Limit(&'static str),
    /// Entry name that must not be joined onto a directory.
    UnsafeName,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Format(msg) | Error::Unsupported(msg) | Error::Limit(msg) => f.write_str(msg),
            Error::ChecksumMismatch => f.write_str("checksum mismatch"),
            Error::UnsafeName => f.write_str("unsafe entry name"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::Format("truncated")
        } else {
            Error::Io(err)
        }
    }
}
