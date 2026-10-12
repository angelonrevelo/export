//! Container identity from magic bytes. Callers do not pass a file extension.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Flac,
    Wav,
    Mp3,
    Aac,
    Opus,
    Ogg,
    Unknown,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Flac => "flac",
            Kind::Wav => "wav",
            Kind::Mp3 => "mp3",
            Kind::Aac => "aac",
            Kind::Opus => "opus",
            Kind::Ogg => "ogg",
            Kind::Unknown => "unknown",
        })
    }
}

pub fn sniff(data: &[u8]) -> Kind {
    if data.len() >= 4 && &data[0..4] == b"fLaC" {
        return Kind::Flac;
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Kind::Wav;
    }
    if data.len() >= 4 && &data[0..4] == b"OggS" {
        let window = if data.len() > 128 { &data[..128] } else { data };
        if window.windows(8).any(|w| w == b"OpusHead") {
            return Kind::Opus;
        }
        return Kind::Ogg;
    }
    if data.len() >= 2 && data[0] == 0xff && (data[1] & 0xf0) == 0xf0 && adts_layer_zero(data) {
        return Kind::Aac;
    }
    if looks_mp3(data) {
        return Kind::Mp3;
    }
    Kind::Unknown
}

fn adts_layer_zero(data: &[u8]) -> bool {
    if data.len() < 2 {
        return false;
    }
    ((data[1] >> 1) & 0x03) == 0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_not_extension() {
        assert_eq!(sniff(b"fLaC"), Kind::Flac);
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&36u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        assert_eq!(sniff(&wav), Kind::Wav);
        assert_eq!(sniff(b"ID3"), Kind::Mp3);
        assert_eq!(sniff(&[0xff, 0xf1, 0x50, 0x80, 0x00, 0x1f, 0xfc]), Kind::Aac);
        let mut opus = b"OggS".to_vec();
        opus.extend_from_slice(&[0u8; 28]);
        opus.extend_from_slice(b"OpusHead");
        assert_eq!(sniff(&opus), Kind::Opus);
        assert_eq!(sniff(b"not audio"), Kind::Unknown);
    }
}
