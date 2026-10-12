/// Container identified from the leading bytes, never from a file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    /// Recognized so it is not fed to the PNG decoder. Not decoded in this crate.
    Webp,
    Unknown,
}

/// Sniff a format from magic bytes.
///
/// PNG is the 8-byte signature. JPEG is `FF D8 FF`. WebP is a RIFF container
/// whose form type is `WEBP`. Anything else is [`Format::Unknown`].
pub fn sniff(bytes: &[u8]) -> Format {
    if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return Format::Png;
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return Format::Jpeg;
    }
    if bytes.len() >= 12
        && &bytes[0..4] == b"RIFF"
        && &bytes[8..12] == b"WEBP"
    {
        return Format::Webp;
    }
    Format::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magics() {
        assert_eq!(
            sniff(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0]),
            Format::Png
        );
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Format::Jpeg);
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend_from_slice(&[0, 1, 2]);
        assert_eq!(sniff(&webp), Format::Webp);
        assert_eq!(sniff(b"GIF89a"), Format::Unknown);
        assert_eq!(sniff(&[0xff, 0xd8]), Format::Unknown);
        // A renamed JPEG is still JPEG. The sniff does not take a path.
        assert_eq!(sniff(&[0xff, 0xd8, 0xff]), Format::Jpeg);
    }
}
