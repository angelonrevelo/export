/// Container identified from the leading bytes, never from a file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Gif87a,
    Gif89a,
    /// ISO BMFF, WebM/Matroska, or AVI. Recognized so it is not decoded.
    Video,
    Unknown,
}

/// Sniff GIF87a, GIF89a, or a video container.
///
/// A video match is only the magic. The rest of the file is not parsed.
pub fn sniff(bytes: &[u8]) -> Format {
    if bytes.len() >= 6 && &bytes[..6] == b"GIF87a" {
        return Format::Gif87a;
    }
    if bytes.len() >= 6 && &bytes[..6] == b"GIF89a" {
        return Format::Gif89a;
    }
    if is_video_magic(bytes) {
        return Format::Video;
    }
    Format::Unknown
}

fn is_video_magic(bytes: &[u8]) -> bool {
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return true;
    }
    if bytes.len() >= 4 && bytes[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        return true;
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"AVI " {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magics() {
        assert_eq!(sniff(b"GIF87a"), Format::Gif87a);
        assert_eq!(sniff(b"GIF89a"), Format::Gif89a);
        assert_eq!(sniff(b"GIF89"), Format::Unknown);
        assert_eq!(sniff(b"GIF87a-extra"), Format::Gif87a);
        assert_eq!(
            sniff(&[0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p']),
            Format::Video
        );
        assert_eq!(sniff(&[0x1a, 0x45, 0xdf, 0xa3, 0x00]), Format::Video);
        let mut avi = b"RIFF\0\0\0\0AVI ".to_vec();
        avi.extend_from_slice(&[1, 2, 3]);
        assert_eq!(sniff(&avi), Format::Video);
        assert_eq!(sniff(b"PNG\r\n"), Format::Unknown);
    }
}
