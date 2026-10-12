//! Subtitle cues for Export. SRT, WebVTT, and ASS/SSA in, any of those out.
//! A binary cue file carries a CRC so a truncated file cannot look valid.

mod ass;
mod cuefile;
mod parse;

pub use ass::{cues_to_ass, parse_ass};
pub use cuefile::{read_cue_file, write_cue_file};
pub use parse::{cues_to_srt, cues_to_vtt, parse_srt, parse_vtt, Cue, Error};

#[cfg(test)]
mod tests {
    use super::{cues_to_srt, cues_to_vtt, parse_srt, parse_vtt, read_cue_file, write_cue_file, Error};

    const SRT: &str = "1\n00:00:01,000 --> 00:00:02,500\nHello\nthere\n\n2\n00:00:03,000 --> 00:00:04,000\nNext\n";

    #[test]
    fn srt_round_trip() {
        let cues = parse_srt(SRT).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 2500);
        assert_eq!(cues[0].text, "Hello\nthere");
        let again = parse_srt(&cues_to_srt(&cues)).unwrap();
        assert_eq!(again, cues);
    }

    #[test]
    fn vtt_round_trip_and_shift() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n";
        let mut cues = parse_vtt(vtt).unwrap();
        cues[0].shift(500);
        assert_eq!(cues[0].start_ms, 1500);
        let text = cues_to_vtt(&cues);
        assert!(text.starts_with("WEBVTT\n"));
        assert_eq!(parse_vtt(&text).unwrap(), cues);
    }

    #[test]
    fn truncated_text_and_truncated_file_error() {
        assert_eq!(parse_srt("1\n00:00:01,000 -->"), Err(Error::Truncated));
        let cues = parse_srt(SRT).unwrap();
        let bytes = write_cue_file(&cues);
        assert_eq!(read_cue_file(&bytes[..bytes.len() - 1]), Err(Error::Truncated));
        let mut flipped = bytes.clone();
        flipped[20] ^= 0x01;
        assert!(read_cue_file(&flipped).is_err());
        assert_eq!(read_cue_file(&bytes).unwrap(), cues);
    }
}
