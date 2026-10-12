/// Failures from sniffing or copying a fragmented MP4 stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The buffer ended before a box or a sample was complete.
    Truncated,
    /// A sample entry is AAC, Opus, or VP9.
    /// Those codecs are sniffed and refused. They are not decoded.
    /// H.264 sample bytes are copied and are not decoded.
    Unsupported,
    /// The bytes are not a usable fragmented MP4 stream.
    Invalid,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::Truncated => "truncated",
            Error::Unsupported => "unsupported",
            Error::Invalid => "invalid",
        })
    }
}

impl std::error::Error for Error {}
