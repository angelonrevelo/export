/// Failures from sniffing or copying a WebM/Matroska stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The buffer ended before an element or block header was complete.
    Truncated,
    /// An element id was empty, reserved, overlong, or wider than 4 bytes.
    BadElementId,
    /// The stream is EBML, and its codec is VP9, Opus, or Vorbis.
    /// Those codecs are sniffed and refused. They are not decoded.
    /// A VP8 block payload is copied and is not decoded.
    Unsupported,
    /// The bytes are not a usable WebM or Matroska stream.
    Invalid,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::Truncated => "truncated",
            Error::BadElementId => "bad element id",
            Error::Unsupported => "unsupported",
            Error::Invalid => "invalid",
        })
    }
}

impl std::error::Error for Error {}
