//! Magic-byte sniff. Extensions are not trusted.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Gif,
    Webp,
    Wav,
    Mp4,
    Pdf,
    Unknown,
}

pub fn sniff(bytes: &[u8]) -> Kind {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Kind::Jpeg;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Kind::Png;
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Kind::Gif;
    }
    if bytes.starts_with(b"%PDF") {
        return Kind::Pdf;
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Kind::Wav;
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Kind::Webp;
    }
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return Kind::Mp4;
    }
    Kind::Unknown
}

#[cfg(test)]
mod tests {
    use super::{sniff, Kind};

    #[test]
    fn ignores_the_name() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0x00]), Kind::Jpeg);
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Kind::Png);
        assert_eq!(sniff(b"%PDF-1.7"), Kind::Pdf);
        assert_eq!(sniff(b"GIF89a"), Kind::Gif);
        let mut wav = b"RIFF....WAVE".to_vec();
        wav[4..8].copy_from_slice(&36u32.to_le_bytes());
        assert_eq!(sniff(&wav), Kind::Wav);
        let mut mp4 = vec![0, 0, 0, 0x18];
        mp4.extend_from_slice(b"ftypisom");
        assert_eq!(sniff(&mp4), Kind::Mp4);
        assert_eq!(sniff(b"not-a-file"), Kind::Unknown);
        assert_eq!(sniff(&[0xFF]), Kind::Unknown);
    }
}
