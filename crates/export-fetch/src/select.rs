//! Format selection for Export URL jobs.
//!
//! Scores metadata only. A size cap drops a known `filesize` without reading
//! the object, so the selector never holds a media buffer.

use crate::error::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaFormat {
    pub id: String,
    pub url: String,
    pub ext: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub abr: Option<u64>,
    pub vbr: Option<u64>,
    pub tbr: Option<u64>,
    pub filesize: Option<u64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub protocol: String,
    pub has_drm: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    Audio,
    Video,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub primary: MediaFormat,
    pub companion_audio: Option<MediaFormat>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Audio,
    Video,
    Images,
}

pub fn job_kind(format: &str) -> Result<JobKind> {
    match format {
        "mp3" | "txt" | "srt" | "vtt" | "notes" | "words" | "kit" | "translate" | "pdf"
        | "docx" | "epub" => Ok(JobKind::Audio),
        "mp4" | "captioned" | "frames" | "subs" => Ok(JobKind::Video),
        "images" => Ok(JobKind::Images),
        other => Err(Error::Unsupported(format!("format {other}"))),
    }
}

pub fn select(formats: &[MediaFormat], want: Want, max_bytes: Option<u64>) -> Result<Pick> {
    let eligible: Vec<&MediaFormat> = formats.iter().filter(|fmt| allowed(fmt, max_bytes)).collect();
    if eligible.is_empty() {
        return Err(Error::Http("no format under the size cap".into()));
    }
    match want {
        Want::Audio => {
            let best = eligible
                .into_iter()
                .filter(|fmt| has_audio(fmt))
                .max_by_key(|fmt| audio_rank(fmt))
                .ok_or_else(|| Error::Http("no audio format".into()))?;
            Ok(Pick {
                primary: best.clone(),
                companion_audio: None,
            })
        }
        Want::Video => select_video(&eligible, max_bytes),
    }
}

fn select_video(eligible: &[&MediaFormat], max_bytes: Option<u64>) -> Result<Pick> {
    let mut progressive: Vec<&&MediaFormat> = eligible
        .iter()
        .filter(|fmt| has_video(fmt) && has_audio(fmt))
        .collect();
    if let Some(best) = progressive
        .iter()
        .max_by_key(|fmt| video_rank(fmt))
        .copied()
    {
        return Ok(Pick {
            primary: (*best).clone(),
            companion_audio: None,
        });
    }
    let _ = &mut progressive;
    let video = eligible
        .iter()
        .copied()
        .filter(|fmt| has_video(fmt))
        .max_by_key(|fmt| video_rank(fmt));
    let audio = eligible
        .iter()
        .copied()
        .filter(|fmt| has_audio(fmt) && !has_video(fmt))
        .max_by_key(|fmt| audio_rank(fmt));
    if let (Some(video), Some(audio)) = (video, audio) {
        if pair_fits(video, audio, max_bytes) {
            return Ok(Pick {
                primary: video.clone(),
                companion_audio: Some(audio.clone()),
            });
        }
    }
    if let Some(video) = video {
        return Ok(Pick {
            primary: video.clone(),
            companion_audio: None,
        });
    }
    Err(Error::Http("no video format".into()))
}

fn pair_fits(video: &MediaFormat, audio: &MediaFormat, max_bytes: Option<u64>) -> bool {
    let Some(max) = max_bytes else {
        return true;
    };
    match (video.filesize, audio.filesize) {
        (Some(a), Some(b)) => a.saturating_add(b) <= max,
        _ => true,
    }
}

fn allowed(fmt: &MediaFormat, max_bytes: Option<u64>) -> bool {
    if fmt.has_drm || fmt.url.is_empty() {
        return false;
    }
    let protocol = fmt.protocol.to_ascii_lowercase();
    if !protocol.is_empty() && protocol != "http" && protocol != "https" {
        return false;
    }
    if !(fmt.url.starts_with("http://") || fmt.url.starts_with("https://")) {
        return false;
    }
    if let (Some(max), Some(size)) = (max_bytes, fmt.filesize) {
        if size > max {
            return false;
        }
    }
    true
}

fn codec_live(value: Option<&String>) -> bool {
    match value.map(String::as_str) {
        None => false,
        Some("none") | Some("") => false,
        Some(_) => true,
    }
}

pub fn has_audio(fmt: &MediaFormat) -> bool {
    if codec_live(fmt.acodec.as_ref()) {
        return true;
    }
    if matches!(fmt.acodec.as_deref(), Some("none")) {
        return false;
    }
    matches!(
        fmt.ext.as_str(),
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" | "m4b"
    )
}

pub fn has_video(fmt: &MediaFormat) -> bool {
    if codec_live(fmt.vcodec.as_ref()) {
        return true;
    }
    if matches!(fmt.vcodec.as_deref(), Some("none")) {
        return false;
    }
    fmt.height.is_some()
        || matches!(fmt.ext.as_str(), "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v")
}

fn audio_rank(fmt: &MediaFormat) -> (u8, u64, u64, u64) {
    let audio_only = u8::from(!has_video(fmt));
    let abr = fmt.abr.or(fmt.tbr).unwrap_or(0);
    let tbr = fmt.tbr.unwrap_or(0);
    let size = fmt.filesize.unwrap_or(u64::MAX);
    (audio_only, abr, tbr, u64::MAX.saturating_sub(size))
}

fn video_rank(fmt: &MediaFormat) -> (u32, u64, u8, u64) {
    let mp4 = u8::from(fmt.ext == "mp4");
    let rate = fmt.tbr.or(fmt.vbr).or(fmt.abr).unwrap_or(0);
    let size = fmt.filesize.unwrap_or(u64::MAX);
    (fmt.height.unwrap_or(0), rate, mp4, u64::MAX.saturating_sub(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(id: &str, ext: &str, height: Option<u32>, abr: Option<u64>, size: Option<u64>) -> MediaFormat {
        MediaFormat {
            id: id.into(),
            url: format!("http://cdn/{id}.{ext}"),
            ext: ext.into(),
            width: None,
            height,
            abr,
            vbr: None,
            tbr: abr,
            filesize: size,
            vcodec: if height.is_some() {
                Some("avc1".into())
            } else {
                Some("none".into())
            },
            acodec: Some(if ext == "m4a" || ext == "webm" && height.is_none() || height.is_some() {
                "mp4a".into()
            } else {
                "none".into()
            }),
            protocol: "http".into(),
            has_drm: false,
        }
    }

    #[test]
    fn best_audio_and_video_honor_the_size_cap() {
        let audio_hi = fmt("a", "webm", None, Some(160_000), Some(1_800));
        let audio_lo = fmt("b", "m4a", None, Some(128_000), Some(1_200));
        let prog_hi = fmt("c", "mp4", Some(720), Some(2_000_000), Some(40_000));
        let prog_lo = fmt("d", "mp4", Some(360), Some(500_000), Some(9_000));
        let mut drm = prog_hi.clone();
        drm.id = "drm".into();
        drm.has_drm = true;
        drm.url = "http://cdn/drm.mp4".into();
        let mut dash = prog_hi.clone();
        dash.id = "dash".into();
        dash.protocol = "m3u8".into();
        dash.url = "http://cdn/x.m3u8".into();
        let all = vec![audio_hi, audio_lo, prog_hi, prog_lo, drm, dash];
        let audio = select(&all, Want::Audio, Some(2_000)).unwrap();
        assert_eq!(audio.primary.id, "a");
        assert!(audio.companion_audio.is_none());
        let capped = select(&all, Want::Audio, Some(1_500)).unwrap();
        assert_eq!(capped.primary.id, "b");
        let video = select(&all, Want::Video, Some(50_000)).unwrap();
        assert_eq!(video.primary.id, "c");
        let smaller = select(&all, Want::Video, Some(10_000)).unwrap();
        assert_eq!(smaller.primary.id, "d");
        assert!(select(&all, Want::Video, Some(100)).is_err());
        assert_eq!(job_kind("notes").unwrap(), JobKind::Audio);
        assert_eq!(job_kind("frames").unwrap(), JobKind::Video);
        assert_eq!(job_kind("images").unwrap(), JobKind::Images);
        assert!(job_kind("nope").is_err());
    }

    #[test]
    fn selection_does_not_allocate_a_body() {
        let huge = fmt("huge", "mp4", Some(2160), Some(20_000_000), Some(u64::MAX / 4));
        let small = fmt("small", "mp4", Some(240), Some(100), Some(50));
        let pick = select(&[huge, small], Want::Video, Some(1000)).unwrap();
        assert_eq!(pick.primary.id, "small");
    }
}
