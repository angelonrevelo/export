//! Minimal PCM WAV. Mono 16-bit little-endian only. No allocation beyond the
//! returned buffer.

/// A parsed mono PCM WAV.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wav {
    pub sample_rate: u32,
    pub pcm: Vec<i16>,
}

/// Why a WAV was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavError {
    Truncated,
    Format,
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavError::Truncated => write!(f, "truncated wav"),
            WavError::Format => write!(f, "not mono pcm16"),
        }
    }
}

impl std::error::Error for WavError {}

/// Encode mono PCM. One allocation for the file bytes.
pub fn write_wav(pcm: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_bytes = pcm.len() * 2;
    let mut out = Vec::with_capacity(44 + data_bytes);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Parse a WAV written by [`write_wav`]. Extra chunks are rejected.
pub fn parse_wav(bytes: &[u8]) -> Result<Wav, WavError> {
    if bytes.len() < 44 {
        return Err(WavError::Truncated);
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" || &bytes[12..16] != b"fmt " {
        return Err(WavError::Format);
    }
    let audio_format = u16::from_le_bytes([bytes[20], bytes[21]]);
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
    if audio_format != 1 || channels != 1 || bits != 16 || &bytes[36..40] != b"data" {
        return Err(WavError::Format);
    }
    let data_bytes = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
    if bytes.len() < 44 + data_bytes || data_bytes % 2 != 0 {
        return Err(WavError::Truncated);
    }
    let mut pcm = Vec::with_capacity(data_bytes / 2);
    let data = &bytes[44..44 + data_bytes];
    for chunk in data.chunks_exact(2) {
        pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(Wav { sample_rate, pcm })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let pcm = [0i16, 1, -2, 32767, -32768];
        let bytes = write_wav(&pcm, 16_000);
        let parsed = parse_wav(&bytes).unwrap();
        assert_eq!(parsed.sample_rate, 16_000);
        assert_eq!(parsed.pcm, pcm);
    }
}
