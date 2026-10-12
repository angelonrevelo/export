use std::fmt;

/// Failure while parsing a playlist or fetching it over cleartext HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The input ended inside a tag or before a segment URI.
    Truncated,
    /// A finished tag or URI is not valid playlist syntax.
    BadTag,
    /// Encryption, cookies, credentials, userinfo, or a non-cleartext URL.
    Unsupported,
    /// Resolved address is loopback (and loopback is not allowed) or link-local.
    Blocked,
    /// The cleartext HTTP/1.1 exchange failed.
    Http(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => write!(f, "truncated playlist"),
            Error::BadTag => write!(f, "bad tag"),
            Error::Unsupported => write!(f, "unsupported"),
            Error::Blocked => write!(f, "blocked address"),
            Error::Http(msg) => write!(f, "http error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
