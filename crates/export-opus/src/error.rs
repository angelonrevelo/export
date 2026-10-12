use std::fmt;

/// Failure from the one-packet encoder or decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The packet ended before the TOC and frame bytes it claimed to hold.
    Truncated,
    /// The TOC byte, or the frame-count structure it introduces, is illegal
    /// or is not the one mono 48 kHz 20 ms frame this crate accepts.
    BadToc,
    /// Multistream, in-band FEC, a stereo packet other than the one prepared
    /// 20 ms packet, or PCM other than silence, the one half-scale 440 Hz
    /// frame, and that frame's quarter-scale right channel.
    Unsupported,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => f.write_str("truncated"),
            Error::BadToc => f.write_str("bad toc"),
            Error::Unsupported => f.write_str("unsupported"),
        }
    }
}

impl std::error::Error for Error {}
