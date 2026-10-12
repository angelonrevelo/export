//! Container identity from magic bytes. Callers do not pass a file extension.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Mp4 { brand: [u8; 4] },
    Mov { brand: [u8; 4] },
    M4a { brand: [u8; 4] },
    Wav,
    Webm,
    Mkv,
    Flac,
    Mp3,
    Ogg,
    Gif,
    Aac,
    Unknown,
}

impl MediaKind {
    pub fn is_bmff(self) -> bool {
        matches!(
            self,
            MediaKind::Mp4 { .. } | MediaKind::Mov { .. } | MediaKind::M4a { .. }
        )
    }
}

impl fmt::Display for MediaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaKind::Mp4 { brand } => write!(f, "mp4/{}", fourcc(brand)),
            MediaKind::Mov { brand } => write!(f, "mov/{}", fourcc(brand)),
            MediaKind::M4a { brand } => write!(f, "m4a/{}", fourcc(brand)),
            MediaKind::Wav => write!(f, "wav"),
            MediaKind::Webm => write!(f, "webm"),
            MediaKind::Mkv => write!(f, "mkv"),
            MediaKind::Flac => write!(f, "flac"),
            MediaKind::Mp3 => write!(f, "mp3"),
            MediaKind::Ogg => write!(f, "ogg"),
            MediaKind::Gif => write!(f, "gif"),
            MediaKind::Aac => write!(f, "aac"),
            MediaKind::Unknown => write!(f, "unknown"),
        }
    }
}

fn fourcc(b: &[u8; 4]) -> String {
    b.iter()
        .map(|c| {
            if c.is_ascii_graphic() {
                *c as char
            } else {
                '.'
            }
        })
        .collect()
}

pub fn sniff(data: &[u8]) -> MediaKind {
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return MediaKind::Wav;
    }
    if data.len() >= 4 && &data[0..4] == b"fLaC" {
        return MediaKind::Flac;
    }
    if data.len() >= 4 && &data[0..4] == b"OggS" {
        return MediaKind::Ogg;
    }
    if data.len() >= 6 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        return MediaKind::Gif;
    }
    if data.len() >= 2 && data[0] == 0xff && (data[1] & 0xf0) == 0xf0 && adts_looks_real(data) {
        return MediaKind::Aac;
    }
    if looks_mp3(data) {
        return MediaKind::Mp3;
    }
    if data.len() >= 4 && data[0] == 0x1a && data[1] == 0x45 && data[2] == 0xdf && data[3] == 0xa3 {
        return sniff_ebml(data);
    }
    if let Some(kind) = sniff_bmff(data) {
        return kind;
    }
    MediaKind::Unknown
}

fn adts_looks_real(data: &[u8]) -> bool {
    if data.len() < 2 {
        return false;
    }
    let layer = (data[1] >> 1) & 0x03;
    layer == 0
}

fn looks_mp3(data: &[u8]) -> bool {
    if data.len() >= 3 && &data[0..3] == b"ID3" {
        return true;
    }
    if data.len() < 4 || data[0] != 0xff || data[1] & 0xe0 != 0xe0 {
        return false;
    }
    let layer = (data[1] >> 1) & 0x03;
    let bitrate = data[2] >> 4;
    let sample = (data[2] >> 2) & 0x03;
    layer != 0 && bitrate != 0 && bitrate != 0x0f && sample != 0x03
}

fn sniff_ebml(data: &[u8]) -> MediaKind {
    // DocType is usually in the first kilobyte. Default to mkv until "webm" shows up.
    let window = &data[..data.len().min(4096)];
    if window.windows(4).any(|w| w == b"webm") {
        MediaKind::Webm
    } else {
        MediaKind::Mkv
    }
}

const BMFF_TYPES: [[u8; 4]; 16] = [
    *b"ftyp", *b"styp", *b"moov", *b"moof", *b"mdat", *b"free", *b"skip", *b"wide", *b"udta",
    *b"meta", *b"sidx", *b"ssix", *b"prft", *b"pdin", *b"meco", *b"uuid",
];

fn is_bmff_type(t: [u8; 4]) -> bool {
    BMFF_TYPES.contains(&t)
}

fn sniff_bmff(data: &[u8]) -> Option<MediaKind> {
    if data.len() < 8 {
        return None;
    }
    let typ = [data[4], data[5], data[6], data[7]];
    if !is_bmff_type(typ) {
        return None;
    }
    let mut off = 0usize;
    for _ in 0..4 {
        if data.len() < off + 8 {
            break;
        }
        let size32 = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        let typ = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        if typ == *b"ftyp" && data.len() >= off + 12 {
            let mut brand = [0u8; 4];
            let n = (data.len() - (off + 8)).min(4);
            brand[..n].copy_from_slice(&data[off + 8..off + 8 + n]);
            return Some(family(brand));
        }
        let size = if size32 == 1 {
            if data.len() < off + 16 {
                break;
            }
            u64::from_be_bytes([
                data[off + 8],
                data[off + 9],
                data[off + 10],
                data[off + 11],
                data[off + 12],
                data[off + 13],
                data[off + 14],
                data[off + 15],
            ])
        } else if size32 == 0 {
            break;
        } else {
            size32 as u64
        };
        if size < 8 || size > (data.len() - off) as u64 {
            break;
        }
        off += size as usize;
    }
    Some(MediaKind::Mp4 { brand: *b"isom" })
}

pub fn family(brand: [u8; 4]) -> MediaKind {
    match &brand {
        b"qt  " => MediaKind::Mov { brand },
        b"M4A " | b"M4B " | b"M4P " => MediaKind::M4a { brand },
        _ => MediaKind::Mp4 { brand },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_what_a_name_would_have_said() {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&36u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        assert_eq!(sniff(&wav), MediaKind::Wav);

        let mut mp4 = Vec::new();
        let body = b"isom\0\0\x02\x00isom";
        let size = (8 + body.len()) as u32;
        mp4.extend_from_slice(&size.to_be_bytes());
        mp4.extend_from_slice(b"ftyp");
        mp4.extend_from_slice(body);
        assert!(matches!(sniff(&mp4), MediaKind::Mp4 { brand } if &brand == b"isom"));

        let mut mov = Vec::new();
        let body = b"qt  \0\0\0\0qt  ";
        let size = (8 + body.len()) as u32;
        mov.extend_from_slice(&size.to_be_bytes());
        mov.extend_from_slice(b"ftyp");
        mov.extend_from_slice(body);
        assert!(matches!(sniff(&mov), MediaKind::Mov { .. }));

        assert_eq!(sniff(b"GIF89a"), MediaKind::Gif);
        assert_eq!(sniff(b"fLaC"), MediaKind::Flac);
        assert_eq!(sniff(b"OggS"), MediaKind::Ogg);
        assert_eq!(
            sniff(&[0x1a, 0x45, 0xdf, 0xa3, b'w', b'e', b'b', b'm']),
            MediaKind::Webm
        );
        assert_eq!(sniff(b"not a media file!!!!"), MediaKind::Unknown);
        assert_eq!(sniff(b"ID3"), MediaKind::Mp3);
    }
}
