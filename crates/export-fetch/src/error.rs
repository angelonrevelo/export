use std::fmt;

/// Recoverable fetch failure. Library paths return this instead of panicking.
#[derive(Debug)]
pub enum Error {
    Ssrf(String),
    Url(String),
    Http(String),
    Tls(String),
    TooLarge { limit: u64, got: u64 },
    NotDirectlyFetchable(String),
    Unsupported(String),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Ssrf(m) => write!(f, "refused: {m}"),
            Error::Url(m) => write!(f, "bad url: {m}"),
            Error::Http(m) => write!(f, "http: {m}"),
            Error::Tls(m) => write!(f, "tls: {m}"),
            Error::TooLarge { limit, got } => {
                write!(f, "size {got} exceeds cap {limit}")
            }
            Error::NotDirectlyFetchable(m) => write!(f, "not directly fetchable: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
