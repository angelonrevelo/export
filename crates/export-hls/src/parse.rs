use crate::error::Error;
use crate::index::index_segments;
use crate::model::{ByteRange, Map, MediaPlaylist, PlaylistType, Segment};
use crate::urlcheck::check_media_uri;

/// Parse a generated HLS media playlist.
///
/// The playlist must start with `#EXTM3U`. A cut-off tag or an `#EXTINF`
/// without its URI is [`Error::Truncated`]. A finished tag with a bad value
/// is [`Error::BadTag`]. `#EXT-X-KEY`, `https` URLs, cookies, credentials,
/// and userinfo are [`Error::Unsupported`].
pub fn parse_media_playlist(input: &str) -> Result<MediaPlaylist, Error> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if input.trim().is_empty() {
        return Err(Error::Truncated);
    }
    let lines = split_lines(input);
    let mut header_at = None;
    for (index, line) in lines.iter().enumerate() {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        header_at = Some((index, text, line.terminated));
        break;
    }
    let (header_index, header, header_terminated) = match header_at {
        Some(found) => found,
        None => return Err(Error::Truncated),
    };
    if header != "#EXTM3U" {
        if !header_terminated && "#EXTM3U".starts_with(header) && header.starts_with('#') {
            return Err(Error::Truncated);
        }
        return Err(Error::BadTag);
    }

    let mut playlist = MediaPlaylist {
        version: None,
        target_duration: None,
        media_sequence: 0,
        discontinuity_sequence: 0,
        playlist_type: None,
        independent_segments: false,
        i_frames_only: false,
        end_list: false,
        init: None,
        segments: Vec::new(),
        spans: Vec::new(),
    };
    let mut saw_media_sequence = false;
    let mut saw_discontinuity_sequence = false;
    let mut pending = Pending::default();
    let mut prev_end: Option<u64> = None;
    let mut current_map: Option<Map> = None;

    for line in lines.iter().skip(header_index + 1) {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        if playlist.end_list {
            if text.starts_with('#') && !text.starts_with("#EXT") {
                continue;
            }
            return Err(contextual(line.terminated, Error::BadTag));
        }
        if text.starts_with('#') {
            if !text.starts_with("#EXT") {
                continue;
            }
            match directive(text, line.terminated)? {
                Directive::Ignored => {}
                Directive::Version(value) => {
                    if playlist.version.is_some() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    playlist.version = Some(value);
                }
                Directive::TargetDuration(value) => {
                    if playlist.target_duration.is_some() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    playlist.target_duration = Some(value);
                }
                Directive::MediaSequence(value) => {
                    if saw_media_sequence || !playlist.segments.is_empty() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    saw_media_sequence = true;
                    playlist.media_sequence = value;
                }
                Directive::DiscontinuitySequence(value) => {
                    if saw_discontinuity_sequence || !playlist.segments.is_empty() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    saw_discontinuity_sequence = true;
                    playlist.discontinuity_sequence = value;
                }
                Directive::PlaylistType(value) => {
                    if playlist.playlist_type.is_some() || !playlist.segments.is_empty() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    playlist.playlist_type = Some(value);
                }
                Directive::IndependentSegments => {
                    if playlist.independent_segments {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    playlist.independent_segments = true;
                }
                Directive::IFramesOnly => {
                    if playlist.i_frames_only || !playlist.segments.is_empty() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    playlist.i_frames_only = true;
                }
                Directive::EndList => {
                    if pending.is_open() {
                        return Err(Error::Truncated);
                    }
                    playlist.end_list = true;
                }
                Directive::Discontinuity => {
                    if pending.discontinuity {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    pending.discontinuity = true;
                }
                Directive::Gap => {
                    if pending.gap {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    pending.gap = true;
                }
                Directive::Inf { duration, title } => {
                    if pending.duration.is_some() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    pending.duration = Some(duration);
                    pending.title = title;
                }
                Directive::ByteRange { length, offset } => {
                    if pending.range.is_some() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    pending.range = Some(PendingRange { length, offset });
                }
                Directive::ProgramDateTime(value) => {
                    if pending.program_date_time.is_some() {
                        return Err(contextual(line.terminated, Error::BadTag));
                    }
                    pending.program_date_time = Some(value);
                }
                Directive::Map(map) => {
                    if playlist.init.is_none() {
                        playlist.init = Some(map.clone());
                    }
                    current_map = Some(map);
                }
            }
            continue;
        }

        let Some(duration) = pending.duration.take() else {
            return Err(contextual(line.terminated, Error::BadTag));
        };
        check_media_uri(text).map_err(|err| contextual(line.terminated, err))?;
        let byte_range = match pending.range.take() {
            Some(range) => Some(resolve_range(range, prev_end)?),
            None => None,
        };
        prev_end = match byte_range {
            Some(range) => Some(range.end().ok_or(Error::BadTag)?),
            None => None,
        };
        let sequence = playlist
            .media_sequence
            .checked_add(playlist.segments.len() as u64)
            .ok_or(Error::BadTag)?;
        playlist.segments.push(Segment {
            uri: text.to_string(),
            duration,
            title: std::mem::take(&mut pending.title),
            byte_range,
            discontinuity: pending.discontinuity,
            gap: pending.gap,
            sequence,
            program_date_time: pending.program_date_time.take(),
            map: current_map.clone(),
        });
        pending.discontinuity = false;
        pending.gap = false;
    }

    if pending.is_open() {
        return Err(Error::Truncated);
    }
    playlist.spans = index_segments(&playlist.segments);
    Ok(playlist)
}

#[derive(Default)]
struct Pending {
    duration: Option<f64>,
    title: String,
    range: Option<PendingRange>,
    discontinuity: bool,
    gap: bool,
    program_date_time: Option<String>,
}

impl Pending {
    fn is_open(&self) -> bool {
        self.duration.is_some()
            || self.range.is_some()
            || self.discontinuity
            || self.gap
            || self.program_date_time.is_some()
    }
}

struct PendingRange {
    length: u64,
    offset: Option<u64>,
}

fn resolve_range(range: PendingRange, prev_end: Option<u64>) -> Result<ByteRange, Error> {
    let offset = match range.offset {
        Some(offset) => offset,
        None => prev_end.ok_or(Error::BadTag)?,
    };
    if offset.checked_add(range.length).is_none() {
        return Err(Error::BadTag);
    }
    Ok(ByteRange {
        length: range.length,
        offset,
    })
}

struct Line<'a> {
    text: &'a str,
    terminated: bool,
}

fn split_lines(input: &str) -> Vec<Line<'_>> {
    let bytes = input.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            lines.push(Line {
                text: &input[start..end],
                terminated: true,
            });
            index += 1;
            start = index;
        } else {
            index += 1;
        }
    }
    if start < bytes.len() {
        let mut end = bytes.len();
        let mut terminated = false;
        if bytes[end - 1] == b'\r' {
            end -= 1;
            terminated = true;
        }
        if start < end || terminated {
            lines.push(Line {
                text: &input[start..end],
                terminated,
            });
        }
    }
    lines
}

enum Directive {
    Version(u32),
    TargetDuration(u64),
    MediaSequence(u64),
    DiscontinuitySequence(u64),
    PlaylistType(PlaylistType),
    IndependentSegments,
    IFramesOnly,
    EndList,
    Discontinuity,
    Gap,
    Inf { duration: f64, title: String },
    ByteRange { length: u64, offset: Option<u64> },
    ProgramDateTime(String),
    Map(Map),
    Ignored,
}

const KNOWN: &[&str] = &[
    "EXTM3U",
    "EXTINF",
    "EXT-X-VERSION",
    "EXT-X-TARGETDURATION",
    "EXT-X-MEDIA-SEQUENCE",
    "EXT-X-DISCONTINUITY-SEQUENCE",
    "EXT-X-DISCONTINUITY",
    "EXT-X-ENDLIST",
    "EXT-X-PLAYLIST-TYPE",
    "EXT-X-BYTERANGE",
    "EXT-X-KEY",
    "EXT-X-SESSION-KEY",
    "EXT-X-MAP",
    "EXT-X-PROGRAM-DATE-TIME",
    "EXT-X-INDEPENDENT-SEGMENTS",
    "EXT-X-I-FRAMES-ONLY",
    "EXT-X-STREAM-INF",
    "EXT-X-I-FRAME-STREAM-INF",
    "EXT-X-MEDIA",
    "EXT-X-SESSION-DATA",
    "EXT-X-GAP",
];

fn directive(line: &str, terminated: bool) -> Result<Directive, Error> {
    let body = line.strip_prefix('#').ok_or(Error::BadTag)?;
    let (name, value) = match body.split_once(':') {
        Some((name, value)) => (name, Some(value)),
        None => (body, None),
    };
    if !valid_tag_name(name) {
        return Err(contextual(terminated, Error::BadTag));
    }
    if is_exact(name) {
        return parse_exact(name, value).map_err(|err| contextual(terminated, err));
    }
    if is_cut_name(name) {
        return Err(if terminated {
            Error::BadTag
        } else {
            Error::Truncated
        });
    }
    if !terminated {
        return Err(Error::Truncated);
    }
    Ok(Directive::Ignored)
}

fn parse_exact(name: &str, value: Option<&str>) -> Result<Directive, Error> {
    match name {
        "EXTM3U" => {
            require_no_value(value)?;
            Err(Error::BadTag)
        }
        "EXTINF" => {
            let raw = value.ok_or(Error::BadTag)?;
            let (duration, title) = raw.split_once(',').ok_or(Error::BadTag)?;
            Ok(Directive::Inf {
                duration: parse_duration(duration.trim())?,
                title: title.to_string(),
            })
        }
        "EXT-X-VERSION" => Ok(Directive::Version(parse_version(require_value(value)?)?)),
        "EXT-X-TARGETDURATION" => Ok(Directive::TargetDuration(parse_u64(require_value(value)?)?)),
        "EXT-X-MEDIA-SEQUENCE" => Ok(Directive::MediaSequence(parse_u64(require_value(value)?)?)),
        "EXT-X-DISCONTINUITY-SEQUENCE" => Ok(Directive::DiscontinuitySequence(parse_u64(
            require_value(value)?,
        )?)),
        "EXT-X-PLAYLIST-TYPE" => {
            let raw = require_value(value)?;
            let kind = match raw {
                "EVENT" => PlaylistType::Event,
                "VOD" => PlaylistType::Vod,
                _ => return Err(Error::BadTag),
            };
            Ok(Directive::PlaylistType(kind))
        }
        "EXT-X-INDEPENDENT-SEGMENTS" => {
            require_no_value(value)?;
            Ok(Directive::IndependentSegments)
        }
        "EXT-X-I-FRAMES-ONLY" => {
            require_no_value(value)?;
            Ok(Directive::IFramesOnly)
        }
        "EXT-X-ENDLIST" => {
            require_no_value(value)?;
            Ok(Directive::EndList)
        }
        "EXT-X-DISCONTINUITY" => {
            require_no_value(value)?;
            Ok(Directive::Discontinuity)
        }
        "EXT-X-GAP" => {
            require_no_value(value)?;
            Ok(Directive::Gap)
        }
        "EXT-X-BYTERANGE" => {
            let (length, offset) = parse_byte_range(require_value(value)?)?;
            Ok(Directive::ByteRange { length, offset })
        }
        "EXT-X-PROGRAM-DATE-TIME" => {
            let raw = require_value(value)?;
            Ok(Directive::ProgramDateTime(raw.to_string()))
        }
        "EXT-X-MAP" => Ok(Directive::Map(parse_map(require_value(value)?)?)),
        "EXT-X-KEY"
        | "EXT-X-SESSION-KEY"
        | "EXT-X-STREAM-INF"
        | "EXT-X-I-FRAME-STREAM-INF"
        | "EXT-X-MEDIA"
        | "EXT-X-SESSION-DATA" => Err(Error::Unsupported),
        _ => Ok(Directive::Ignored),
    }
}

fn parse_map(value: &str) -> Result<Map, Error> {
    let attrs = parse_attributes(value)?;
    let uri = require_attr(&attrs, "URI")?;
    check_media_uri(uri)?;
    let byte_range = match optional_attr(&attrs, "BYTERANGE")? {
        Some(raw) => {
            let (length, offset) = parse_byte_range(raw)?;
            let offset = offset.unwrap_or(0);
            if offset.checked_add(length).is_none() {
                return Err(Error::BadTag);
            }
            Some(ByteRange { length, offset })
        }
        None => None,
    };
    Ok(Map {
        uri: uri.to_string(),
        byte_range,
    })
}

fn parse_attributes(input: &str) -> Result<Vec<(String, String)>, Error> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index] == b' ' {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let name_start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_uppercase()
                || bytes[index].is_ascii_digit()
                || bytes[index] == b'-')
        {
            index += 1;
        }
        if index == name_start || index >= bytes.len() || bytes[index] != b'=' {
            return Err(Error::BadTag);
        }
        let name = input[name_start..index].to_string();
        index += 1;
        if index >= bytes.len() {
            return Err(Error::BadTag);
        }
        let value = if bytes[index] == b'"' {
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            if index >= bytes.len() {
                return Err(Error::BadTag);
            }
            let value = input[start..index].to_string();
            index += 1;
            value
        } else {
            let start = index;
            while index < bytes.len() && bytes[index] != b',' {
                index += 1;
            }
            input[start..index].trim().to_string()
        };
        if value.is_empty() {
            return Err(Error::BadTag);
        }
        out.push((name, value));
        if index < bytes.len() {
            if bytes[index] != b',' {
                return Err(Error::BadTag);
            }
            index += 1;
        }
    }
    if out.is_empty() {
        return Err(Error::BadTag);
    }
    Ok(out)
}

fn require_attr<'a>(attrs: &'a [(String, String)], name: &str) -> Result<&'a str, Error> {
    let mut found = None;
    for (key, value) in attrs {
        if key == name {
            if found.is_some() {
                return Err(Error::BadTag);
            }
            found = Some(value.as_str());
        }
    }
    found.ok_or(Error::BadTag)
}

fn optional_attr<'a>(attrs: &'a [(String, String)], name: &str) -> Result<Option<&'a str>, Error> {
    let mut found = None;
    for (key, value) in attrs {
        if key == name {
            if found.is_some() {
                return Err(Error::BadTag);
            }
            found = Some(value.as_str());
        }
    }
    Ok(found)
}

fn parse_byte_range(value: &str) -> Result<(u64, Option<u64>), Error> {
    let (length_text, offset_text) = match value.split_once('@') {
        Some((length, offset)) => (length, Some(offset)),
        None => (value, None),
    };
    let length = parse_u64(length_text.trim())?;
    if length == 0 {
        return Err(Error::BadTag);
    }
    let offset = match offset_text {
        Some(offset) => Some(parse_u64(offset.trim())?),
        None => None,
    };
    Ok((length, offset))
}

fn parse_version(value: &str) -> Result<u32, Error> {
    let parsed = parse_u64(value)?;
    if parsed == 0 || parsed > u32::MAX as u64 {
        return Err(Error::BadTag);
    }
    Ok(parsed as u32)
}

fn parse_u64(value: &str) -> Result<u64, Error> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::BadTag);
    }
    value.parse().map_err(|_| Error::BadTag)
}

fn parse_duration(value: &str) -> Result<f64, Error> {
    if value.is_empty() || !value.bytes().any(|b| b.is_ascii_digit()) {
        return Err(Error::BadTag);
    }
    let mut dots = 0;
    for byte in value.bytes() {
        if byte == b'.' {
            dots += 1;
            if dots > 1 {
                return Err(Error::BadTag);
            }
        } else if !byte.is_ascii_digit() {
            return Err(Error::BadTag);
        }
    }
    let parsed: f64 = value.parse().map_err(|_| Error::BadTag)?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(Error::BadTag);
    }
    Ok(parsed)
}

fn require_value(value: Option<&str>) -> Result<&str, Error> {
    match value {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Err(Error::BadTag)
            } else {
                Ok(trimmed)
            }
        }
        None => Err(Error::BadTag),
    }
}

fn require_no_value(value: Option<&str>) -> Result<(), Error> {
    if value.is_some() {
        Err(Error::BadTag)
    } else {
        Ok(())
    }
}

fn valid_tag_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(byte) if byte.is_ascii_uppercase() => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_exact(name: &str) -> bool {
    KNOWN.contains(&name)
}

fn is_cut_name(name: &str) -> bool {
    KNOWN
        .iter()
        .any(|known| known.starts_with(name) && *known != name)
}

fn contextual(terminated: bool, err: Error) -> Error {
    match err {
        Error::BadTag if !terminated => Error::Truncated,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::COALESCE_GAP_BYTES;

    fn parse_ok(text: &str) -> MediaPlaylist {
        parse_media_playlist(text).unwrap()
    }

    #[test]
    fn parses_extinf_segments() {
        let playlist = parse_ok(
            "#EXTM3U\n\
             #EXT-X-VERSION:3\n\
             #EXT-X-TARGETDURATION:10\n\
             #EXT-X-MEDIA-SEQUENCE:4\n\
             #EXT-X-PLAYLIST-TYPE:VOD\n\
             #EXTINF:9.009,title\n\
             seg0.ts\n\
             #EXTINF:8,\n\
             seg1.ts\n\
             #EXT-X-ENDLIST\n",
        );
        assert_eq!(playlist.version, Some(3));
        assert_eq!(playlist.target_duration, Some(10));
        assert_eq!(playlist.media_sequence, 4);
        assert_eq!(playlist.playlist_type, Some(PlaylistType::Vod));
        assert!(playlist.end_list);
        assert_eq!(playlist.segments.len(), 2);
        assert_eq!(playlist.segments[0].uri, "seg0.ts");
        assert_eq!(playlist.segments[0].title, "title");
        assert_eq!(playlist.segments[0].sequence, 4);
        assert_eq!(playlist.segments[1].sequence, 5);
        assert!((playlist.segments[0].duration - 9.009).abs() < 1e-9);
    }

    #[test]
    fn accepts_crlf_and_a_missing_final_newline() {
        let playlist = parse_ok("#EXTM3U\r\n#EXTINF:1,\r\nseg.ts");
        assert_eq!(playlist.segments.len(), 1);
        assert_eq!(playlist.segments[0].uri, "seg.ts");
    }

    #[test]
    fn implicit_byte_range_continues_from_the_previous_end() {
        let playlist = parse_ok(
            "#EXTM3U\n\
             #EXTINF:1,\n\
             #EXT-X-BYTERANGE:100@0\n\
             file.ts\n\
             #EXTINF:1,\n\
             #EXT-X-BYTERANGE:40\n\
             file.ts\n",
        );
        assert_eq!(
            playlist.segments[1].byte_range,
            Some(ByteRange {
                length: 40,
                offset: 100,
            })
        );
    }

    #[test]
    fn coalesces_ranges_with_a_4096_hole_and_keeps_a_4097_hole_split() {
        let tight = parse_ok(&ranged_playlist(100, 100 + COALESCE_GAP_BYTES));
        assert_eq!(tight.segments.len(), 2);
        assert_eq!(tight.spans.len(), 1);
        assert_eq!(
            tight.spans[0].byte_range,
            Some(ByteRange {
                offset: 0,
                length: 100 + COALESCE_GAP_BYTES + 50,
            })
        );
        assert_eq!(tight.spans[0].segments, vec![0, 1]);

        let loose = parse_ok(&ranged_playlist(100, 100 + COALESCE_GAP_BYTES + 1));
        assert_eq!(loose.spans.len(), 2);
    }

    #[test]
    fn different_uris_do_not_coalesce() {
        let playlist = parse_ok(
            "#EXTM3U\n\
             #EXTINF:1,\n\
             #EXT-X-BYTERANGE:100@0\n\
             a.ts\n\
             #EXTINF:1,\n\
             #EXT-X-BYTERANGE:100@100\n\
             b.ts\n",
        );
        assert_eq!(playlist.spans.len(), 2);
    }

    #[test]
    fn truncated_playlist_and_bad_tag() {
        assert_eq!(parse_media_playlist(""), Err(Error::Truncated));
        assert_eq!(parse_media_playlist("#EXTM"), Err(Error::Truncated));
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXTINF:1,\n"),
            Err(Error::Truncated)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXT-X-TAR"),
            Err(Error::Truncated)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXTINF:nope,\nseg.ts\n"),
            Err(Error::BadTag)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXT-X-BYTERANGE:0@0\n#EXTINF:1,\nseg.ts\n"),
            Err(Error::BadTag)
        );
        assert_eq!(parse_media_playlist("not a playlist\n"), Err(Error::BadTag));
    }

    #[test]
    fn encryption_https_cookies_and_userinfo_are_unsupported() {
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXT-X-KEY:METHOD=NONE\n"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"http://k\"\n"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXTINF:1,\nhttps://cdn.example/a.ts\n"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXTINF:1,\nhttp://user:pass@cdn.example/a.ts\n"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXTINF:1,\nhttp://cdn.example/a.ts?cookie=1\n"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXT-X-MAP:URI=\"https://cdn.example/init.mp4\"\n"),
            Err(Error::Unsupported)
        );
    }

    #[test]
    fn parses_map_and_ignores_comments() {
        let playlist = parse_ok(
            "#EXTM3U\n\
             # a comment\n\
             #EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"720@0\"\n\
             #EXTINF:4,\n\
             http://cdn.example/seg.ts\n",
        );
        let init = playlist.init.unwrap();
        assert_eq!(init.uri, "init.mp4");
        assert_eq!(
            init.byte_range,
            Some(ByteRange {
                length: 720,
                offset: 0,
            })
        );
        assert_eq!(playlist.segments[0].uri, "http://cdn.example/seg.ts");
    }

    #[test]
    fn generated_playlist_collapses_to_one_span() {
        let text = generated_playlist(32);
        let playlist = parse_ok(&text);
        assert_eq!(playlist.segments.len(), 32);
        assert_eq!(playlist.spans.len(), 1);
        let end = 31 * 10_000 + 9_000;
        assert_eq!(
            playlist.spans[0].byte_range,
            Some(ByteRange {
                offset: 0,
                length: end,
            })
        );
    }

    fn ranged_playlist(first_len: u64, second_off: u64) -> String {
        format!(
            "#EXTM3U\n\
             #EXTINF:1,\n\
             #EXT-X-BYTERANGE:{first_len}@0\n\
             file.ts\n\
             #EXTINF:1,\n\
             #EXT-X-BYTERANGE:50@{second_off}\n\
             file.ts\n"
        )
    }

    fn generated_playlist(count: usize) -> String {
        let mut text = String::from(
            "#EXTM3U\n\
             #EXT-X-VERSION:3\n\
             #EXT-X-TARGETDURATION:6\n\
             #EXT-X-MEDIA-SEQUENCE:0\n\
             #EXT-X-PLAYLIST-TYPE:VOD\n",
        );
        for index in 0..count {
            let offset = index as u64 * 10_000;
            text.push_str("#EXTINF:6.000,\n");
            text.push_str(&format!("#EXT-X-BYTERANGE:9000@{offset}\n"));
            text.push_str("media.ts\n");
        }
        text.push_str("#EXT-X-ENDLIST\n");
        text
    }
}
