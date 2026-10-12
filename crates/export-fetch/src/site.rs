//! Site adapters. Direct media files, HTML galleries, and progressive
//! YouTube / oEmbed pages. Cipher URLs and DRM are refused. No cookies.

use crate::error::{Error, Result};
use crate::http::{self, GetCfg};
use crate::policy::{self, Url};
use crate::select::{self, job_kind, JobKind, MediaFormat, Want};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PAGE_CAP: u64 = 2_000_000;

#[derive(Debug)]
pub struct ResolvedMedia {
    pub title: String,
    pub formats: Vec<MediaFormat>,
    pub images: Vec<String>,
    pub oembed: Option<String>,
}

impl ResolvedMedia {
    fn empty() -> Self {
        Self {
            title: String::new(),
            formats: Vec::new(),
            images: Vec::new(),
            oembed: None,
        }
    }
}

pub struct JobOutput {
    pub title: String,
    pub paths: Vec<PathBuf>,
    pub kind: JobKind,
}

pub trait SiteAdapter {
    fn name(&self) -> &'static str;
    fn matches(&self, url: &str) -> bool;
    fn resolve(&self, url: &str, cfg: &GetCfg) -> Result<ResolvedMedia>;
}

pub struct DirectAdapter;
pub struct GalleryAdapter;
pub struct WatchAdapter;
pub struct YoutubeAdapter;

impl SiteAdapter for DirectAdapter {
    fn name(&self) -> &'static str {
        "direct"
    }

    fn matches(&self, url: &str) -> bool {
        policy::parse_url(url)
            .map(|url| url.scheme == "http")
            .unwrap_or(false)
    }

    fn resolve(&self, url: &str, cfg: &GetCfg) -> Result<ResolvedMedia> {
        match probe(cfg, url)? {
            Some(fmt) => Ok(ResolvedMedia {
                title: policy::sanitize_stem(file_stem(url)),
                formats: vec![fmt],
                images: Vec::new(),
                oembed: None,
            }),
            None => Ok(ResolvedMedia::empty()),
        }
    }
}

impl SiteAdapter for GalleryAdapter {
    fn name(&self) -> &'static str {
        "gallery"
    }

    fn matches(&self, url: &str) -> bool {
        DirectAdapter.matches(url)
    }

    fn resolve(&self, url: &str, cfg: &GetCfg) -> Result<ResolvedMedia> {
        if let Some(fmt) = probe(cfg, url)? {
            if is_image_ext(&fmt.ext) {
                return Ok(ResolvedMedia {
                    title: policy::sanitize_stem(file_stem(url)),
                    formats: Vec::new(),
                    images: vec![fmt.url],
                    oembed: None,
                });
            }
        }
        let (head, body) = http::get_capped(cfg, url, PAGE_CAP)?;
        let base = policy::parse_url(&head.final_url)?;
        let mut resolved = parse_html_media(&String::from_utf8_lossy(&body), &base);
        if resolved.title.is_empty() {
            resolved.title = html_title(&String::from_utf8_lossy(&body));
        }
        resolved.formats.clear();
        resolved.images.retain(|item| item.starts_with("http://"));
        Ok(resolved)
    }
}

impl SiteAdapter for WatchAdapter {
    fn name(&self) -> &'static str {
        "watch"
    }

    fn matches(&self, url: &str) -> bool {
        DirectAdapter.matches(url) && !YoutubeAdapter.matches(url)
    }

    fn resolve(&self, url: &str, cfg: &GetCfg) -> Result<ResolvedMedia> {
        if let Some(fmt) = probe(cfg, url)? {
            return Ok(ResolvedMedia {
                title: policy::sanitize_stem(file_stem(url)),
                formats: vec![fmt],
                images: Vec::new(),
                oembed: None,
            });
        }
        let (head, body) = http::get_capped(cfg, url, PAGE_CAP)?;
        let text = String::from_utf8_lossy(&body).into_owned();
        let base = policy::parse_url(&head.final_url)?;
        let mut resolved = parse_html_media(&text, &base);
        if resolved.title.is_empty() {
            resolved.title = html_title(&text);
        }
        if let Some(oembed) = resolved.oembed.clone() {
            if oembed.starts_with("http://") {
                if let Ok((_, bytes)) = http::get_capped(cfg, &oembed, 256 * 1024) {
                    apply_oembed(&mut resolved, &String::from_utf8_lossy(&bytes));
                }
            }
        }
        resolved
            .formats
            .retain(|fmt| fmt.url.starts_with("http://") || fmt.url.starts_with("https://"));
        if resolved.formats.is_empty() {
            return Err(Error::NotDirectlyFetchable(
                "watch page has no direct media url".into(),
            ));
        }
        Ok(resolved)
    }
}

impl SiteAdapter for YoutubeAdapter {
    fn name(&self) -> &'static str {
        "youtube"
    }

    fn matches(&self, url: &str) -> bool {
        policy::parse_url(url)
            .map(|parsed| host_is_youtube(&parsed.host))
            .unwrap_or(false)
    }

    fn resolve(&self, url: &str, cfg: &GetCfg) -> Result<ResolvedMedia> {
        let parsed = policy::parse_url(url)?;
        if parsed.scheme != "http" {
            return Err(Error::Unsupported(
                "cleartext http only; this crate has no TLS and does not decipher YouTube signatures"
                    .into(),
            ));
        }
        if !host_is_youtube(&parsed.host) && video_id(url).is_none() {
            return Err(Error::Unsupported("not a youtube url".into()));
        }
        let (_head, body) = http::get_capped(cfg, url, PAGE_CAP)?;
        from_watch_html(&String::from_utf8_lossy(&body))
    }
}

impl YoutubeAdapter {
    pub fn from_html(html: &str) -> Result<ResolvedMedia> {
        from_watch_html(html)
    }
}

pub fn video_id(url: &str) -> Option<String> {
    let parsed = policy::parse_url(url).ok()?;
    let host = parsed.host.to_ascii_lowercase();
    if host == "youtu.be" || host.ends_with(".youtu.be") {
        let id = parsed.path.trim_start_matches('/');
        let id = id.split(['/', '?']).next().unwrap_or("");
        return nonempty_id(id);
    }
    if !host_is_youtube(&host) {
        return None;
    }
    if let Some(query) = parsed.path.split_once('?').map(|(_, q)| q) {
        for pair in query.split('&') {
            if let Some(id) = pair.strip_prefix("v=") {
                return nonempty_id(id);
            }
        }
    }
    for marker in ["/embed/", "/shorts/", "/live/", "/v/"] {
        if let Some(rest) = parsed.path.split(marker).nth(1) {
            return nonempty_id(rest.split(['/', '?']).next().unwrap_or(""));
        }
    }
    None
}

fn nonempty_id(id: &str) -> Option<String> {
    let id = id.trim();
    if id.len() >= 6 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        Some(id.to_string())
    } else {
        None
    }
}

fn host_is_youtube(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = host.to_ascii_lowercase();
    host == "youtu.be"
        || host == "youtube.com"
        || host == "youtube-nocookie.com"
        || host.ends_with(".youtube.com")
        || host.ends_with(".youtube-nocookie.com")
}

pub fn fetch_job(url: &str, format: &str, cfg: &GetCfg, dest: &Path) -> Result<JobOutput> {
    let kind = job_kind(format)?;
    std::fs::create_dir_all(dest)?;
    match kind {
        JobKind::Images => fetch_images(url, cfg, dest),
        JobKind::Audio | JobKind::Video => fetch_av(url, cfg, dest, kind),
    }
}

fn fetch_images(url: &str, cfg: &GetCfg, dest: &Path) -> Result<JobOutput> {
    let resolved = GalleryAdapter.resolve(url, cfg)?;
    if resolved.images.is_empty() {
        return Err(Error::Http("gallery page has no image url".into()));
    }
    let mut paths = Vec::new();
    for (index, image) in resolved.images.iter().take(32).enumerate() {
        let ext = policy::extension_of(image).unwrap_or_else(|| "img".into());
        let path = dest.join(format!("{:03}.{ext}", index + 1));
        http::download_file(cfg, image, &path)?;
        if !is_image_ext(&ext) && sniff_file(&path).map(is_image_ext).unwrap_or(false) {
            let sniffed = sniff_file(&path).unwrap();
            let renamed = dest.join(format!("{:03}.{sniffed}", index + 1));
            std::fs::rename(&path, &renamed)?;
            paths.push(renamed);
        } else {
            paths.push(path);
        }
    }
    if paths.len() <= 1 {
        return Ok(JobOutput {
            title: resolved.title,
            paths,
            kind: JobKind::Images,
        });
    }
    let zip_path = dest.join("images.zip");
    write_stored_zip(&paths, &zip_path)?;
    for path in &paths {
        let _ = std::fs::remove_file(path);
    }
    Ok(JobOutput {
        title: if resolved.title.is_empty() {
            "images".into()
        } else {
            resolved.title
        },
        paths: vec![zip_path],
        kind: JobKind::Images,
    })
}

fn fetch_av(url: &str, cfg: &GetCfg, dest: &Path, kind: JobKind) -> Result<JobOutput> {
    let resolved = resolve_av(url, cfg)?;
    let want = match kind {
        JobKind::Audio => Want::Audio,
        JobKind::Video => Want::Video,
        JobKind::Images => Want::Video,
    };
    let pick = select::select(&resolved.formats, want, cfg.max_bytes)?;
    let mut paths = Vec::new();
    let primary = dest.join(format!("media.{}", pick.primary.ext));
    http::download_file(cfg, &pick.primary.url, &primary)?;
    paths.push(primary);
    if let Some(audio) = &pick.companion_audio {
        let path = dest.join(format!("audio.{}", audio.ext));
        http::download_file(cfg, &audio.url, &path)?;
        paths.push(path);
    }
    Ok(JobOutput {
        title: resolved.title,
        paths,
        kind,
    })
}

fn resolve_av(url: &str, cfg: &GetCfg) -> Result<ResolvedMedia> {
    if YoutubeAdapter.matches(url) {
        return YoutubeAdapter.resolve(url, cfg);
    }
    if let Some(fmt) = probe(cfg, url)? {
        return Ok(ResolvedMedia {
            title: policy::sanitize_stem(file_stem(url)),
            formats: vec![fmt],
            images: Vec::new(),
            oembed: None,
        });
    }
    WatchAdapter.resolve(url, cfg)
}

fn probe(cfg: &GetCfg, url: &str) -> Result<Option<MediaFormat>> {
    let head = http::head(cfg, url)?;
    let ct = head.content_type.clone().unwrap_or_default();
    let ext = ext_from_content_type(&ct).or_else(|| policy::extension_of(&head.final_url));
    if !is_media_type(&ct) && !is_media_ext(ext.as_deref()) {
        return Ok(None);
    }
    let ext = ext.unwrap_or_else(|| "bin".into());
    let (vcodec, acodec) = codecs_for_type(&ct, &ext);
    Ok(Some(MediaFormat {
        id: "direct".into(),
        url: head.final_url,
        ext,
        width: None,
        height: None,
        abr: None,
        vbr: None,
        tbr: None,
        filesize: head.content_length,
        vcodec,
        acodec,
        protocol: "http".into(),
        has_drm: false,
    }))
}

fn codecs_for_type(ct: &str, ext: &str) -> (Option<String>, Option<String>) {
    let base = ct.split(';').next().unwrap_or("").trim();
    if base.starts_with("audio/") || is_audio_ext(ext) {
        return (Some("none".into()), Some("audio".into()));
    }
    if base.starts_with("video/") || is_video_ext(ext) {
        return (Some("video".into()), Some("audio".into()));
    }
    (None, None)
}

pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some("wav");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return Some("jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some("mp4");
    }
    if bytes.starts_with(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
    {
        return Some("mp3");
    }
    if bytes.starts_with(b"OggS") {
        return Some("ogg");
    }
    if bytes.starts_with(b"fLaC") {
        return Some("flac");
    }
    if bytes.starts_with(b"WEBVTT") {
        return Some("vtt");
    }
    None
}

fn sniff_file(path: &Path) -> Option<&'static str> {
    let mut file = File::open(path).ok()?;
    let mut buf = [0u8; 16];
    let n = file.read(&mut buf).ok()?;
    sniff(&buf[..n])
}

fn ext_from_content_type(ct: &str) -> Option<String> {
    let base = ct.split(';').next()?.trim().to_ascii_lowercase();
    let ext = match base.as_str() {
        "video/mp4" => "mp4",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/mpeg" => "mp3",
        "audio/webm" | "video/webm" => "webm",
        "video/quicktime" => "mov",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "text/vtt" => "vtt",
        "application/x-subrip" => "srt",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" => "flac",
        "audio/ogg" | "application/ogg" => "ogg",
        _ => return None,
    };
    Some(ext.to_string())
}

fn is_media_type(ct: &str) -> bool {
    let base = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    base.starts_with("audio/")
        || base.starts_with("video/")
        || base.starts_with("image/")
        || base == "text/vtt"
        || base == "application/ogg"
}

fn is_media_ext(ext: Option<&str>) -> bool {
    ext.map(|ext| is_audio_ext(ext) || is_video_ext(ext) || is_image_ext(ext) || ext == "vtt" || ext == "srt")
        .unwrap_or(false)
}

fn is_audio_ext(ext: &str) -> bool {
    matches!(ext, "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" | "m4b")
}

fn is_video_ext(ext: &str) -> bool {
    matches!(ext, "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v")
}

fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "bmp" | "tif" | "tiff"
    )
}

fn file_stem(url: &str) -> &str {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or("download");
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

pub fn parse_html_media(html: &str, base: &Url) -> ResolvedMedia {
    let mut resolved = ResolvedMedia::empty();
    resolved.title = html_title(html);
    let mut seen = std::collections::BTreeSet::new();
    walk_tags(html, |tag, attrs| {
        let name = tag_name(tag);
        let attr = |key: &str| {
            attrs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
        };
        if name == "meta" {
            let prop = attr("property").or_else(|| attr("name")).unwrap_or("");
            let content = attr("content").unwrap_or("");
            if prop.eq_ignore_ascii_case("og:title") && resolved.title.is_empty() {
                resolved.title = content.to_string();
            }
            if prop.eq_ignore_ascii_case("og:video")
                || prop.eq_ignore_ascii_case("og:video:url")
                || prop.eq_ignore_ascii_case("og:video:secure_url")
            {
                push_format(&mut resolved, base, content);
            }
            if prop.eq_ignore_ascii_case("og:image") || prop.eq_ignore_ascii_case("og:image:url") {
                push_image(&mut resolved, &mut seen, base, content);
            }
        } else if name == "link" {
            let rel = attr("rel").unwrap_or("");
            let typ = attr("type").unwrap_or("");
            if rel.eq_ignore_ascii_case("alternate")
                && typ.eq_ignore_ascii_case("application/json+oembed")
            {
                if let Some(href) = attr("href") {
                    if let Ok(joined) = policy::join_url(base, href) {
                        resolved.oembed = Some(joined.to_string_url());
                    }
                }
            }
        } else if name == "img" || name == "source" {
            if let Some(src) = attr("src") {
                push_image(&mut resolved, &mut seen, base, src);
            }
            if let Some(srcset) = attr("srcset") {
                for part in srcset.split(',') {
                    if let Some(url) = part.split_whitespace().next() {
                        push_image(&mut resolved, &mut seen, base, url);
                    }
                }
            }
        } else if name == "a" {
            if let Some(href) = attr("href") {
                if policy::extension_of(href).as_deref().map(is_image_ext).unwrap_or(false) {
                    push_image(&mut resolved, &mut seen, base, href);
                }
            }
        }
    });
    resolved
}

fn push_format(resolved: &mut ResolvedMedia, base: &Url, raw: &str) {
    let Ok(url) = policy::join_url(base, raw) else {
        return;
    };
    if url.scheme != "http" && url.scheme != "https" {
        return;
    }
    let ext = policy::extension_of(&url.path).unwrap_or_else(|| "mp4".into());
    let (vcodec, acodec) = if is_audio_ext(&ext) {
        (Some("none".into()), Some("audio".into()))
    } else {
        (Some("video".into()), Some("audio".into()))
    };
    resolved.formats.push(MediaFormat {
        id: format!("og-{}", resolved.formats.len()),
        url: url.to_string_url(),
        ext,
        width: None,
        height: None,
        abr: None,
        vbr: None,
        tbr: None,
        filesize: None,
        vcodec,
        acodec,
        protocol: url.scheme,
        has_drm: false,
    });
}

fn push_image(
    resolved: &mut ResolvedMedia,
    seen: &mut std::collections::BTreeSet<String>,
    base: &Url,
    raw: &str,
) {
    if raw.starts_with("data:") || raw.starts_with("javascript:") {
        return;
    }
    let Ok(url) = policy::join_url(base, raw) else {
        return;
    };
    if url.scheme != "http" && url.scheme != "https" {
        return;
    }
    let text = url.to_string_url();
    if seen.insert(text.clone()) {
        resolved.images.push(text);
    }
}

fn tag_name(tag: &str) -> String {
    let trimmed = tag.trim_start_matches('<').trim_start();
    let end = trimmed
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .unwrap_or(trimmed.len());
    trimmed[..end].to_ascii_lowercase()
}

fn walk_tags(html: &str, mut visit: impl FnMut(&str, &[(String, String)])) {
    let bytes = html.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && bytes[index] != b'>' {
            index += 1;
        }
        if index < bytes.len() {
            index += 1;
        }
        let tag = &html[start..index];
        if tag.starts_with("</") || tag.starts_with("<!") {
            continue;
        }
        let attrs = parse_attrs(tag);
        visit(tag, &attrs);
    }
}

fn parse_attrs(tag: &str) -> Vec<(String, String)> {
    let bytes = tag.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index] != b' ' && bytes[index] != b'>' {
        index += 1;
    }
    let mut out = Vec::new();
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'>' || bytes[index] == b'/' {
            break;
        }
        let name_start = index;
        while index < bytes.len()
            && bytes[index] != b'='
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'>'
        {
            index += 1;
        }
        let name = tag[name_start..index].to_ascii_lowercase();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let value = if bytes[index] == b'"' || bytes[index] == b'\'' {
            let quote = bytes[index];
            index += 1;
            let value_start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            let value = tag[value_start..index].to_string();
            if index < bytes.len() {
                index += 1;
            }
            value
        } else {
            let value_start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>' {
                index += 1;
            }
            tag[value_start..index].to_string()
        };
        out.push((name, value));
    }
    out
}

fn html_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title>") else {
        return String::new();
    };
    let rest = &html[start + 7..];
    let lower_rest = rest.to_ascii_lowercase();
    let end = lower_rest.find("</title>").unwrap_or(rest.len().min(180));
    rest[..end].trim().to_string()
}

fn apply_oembed(resolved: &mut ResolvedMedia, json: &str) {
    let Ok(value) = parse_json(json) else {
        return;
    };
    if resolved.title.is_empty() {
        if let Some(title) = value.get("title").and_then(Json::as_str) {
            resolved.title = title.to_string();
        }
    }
    if let Some(url) = value.get("url").and_then(Json::as_str) {
        if url.starts_with("http://") || url.starts_with("https://") {
            if let Ok(parsed) = policy::parse_url(url) {
                push_format(resolved, &parsed, url);
            }
        }
    }
    if let Some(url) = value.get("thumbnail_url").and_then(Json::as_str) {
        if url.starts_with("http://") || url.starts_with("https://") {
            resolved.images.push(url.to_string());
        }
    }
}

pub fn from_watch_html(html: &str) -> Result<ResolvedMedia> {
    let Some(json) = extract_json(html, "ytInitialPlayerResponse") else {
        return Err(Error::NotDirectlyFetchable(
            "no player response on the watch page".into(),
        ));
    };
    let mut resolved = ResolvedMedia::empty();
    if let Some(title) = json
        .get("videoDetails")
        .and_then(|v| v.get("title"))
        .and_then(Json::as_str)
    {
        resolved.title = title.to_string();
    }
    if let Some(status) = json.get("playabilityStatus") {
        let code = status.get("status").and_then(Json::as_str).unwrap_or("");
        let reason = status.get("reason").and_then(Json::as_str).unwrap_or("");
        let blob = format!("{code} {reason}").to_ascii_lowercase();
        if blob.contains("drm") {
            return Err(Error::NotDirectlyFetchable(
                "drm is not bypassed".into(),
            ));
        }
        if code.eq_ignore_ascii_case("LOGIN_REQUIRED") || blob.contains("sign in") {
            return Err(Error::NotDirectlyFetchable(
                "login required; this crate does not collect credentials".into(),
            ));
        }
    }
    let Some(streaming) = json.get("streamingData") else {
        return Err(Error::NotDirectlyFetchable(
            "no streaming data".into(),
        ));
    };
    let mut saw_cipher = false;
    for key in ["formats", "adaptiveFormats"] {
        let Some(list) = streaming.get(key).and_then(Json::as_array) else {
            continue;
        };
        for item in list {
            let direct = item.get("url").and_then(Json::as_str).unwrap_or("");
            if direct.is_empty() {
                if item.get("signatureCipher").is_some() || item.get("cipher").is_some() {
                    saw_cipher = true;
                }
                continue;
            }
            if let Some(fmt) = format_from_player(item, direct) {
                resolved.formats.push(fmt);
            }
        }
    }
    if resolved.formats.is_empty() {
        if saw_cipher {
            return Err(Error::NotDirectlyFetchable(
                "signature cipher is not solved; this crate has no JS runtime".into(),
            ));
        }
        return Err(Error::NotDirectlyFetchable(
            "no progressive or direct media url".into(),
        ));
    }
    Ok(resolved)
}

fn format_from_player(item: &Json, url: &str) -> Option<MediaFormat> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let mime = item.get("mimeType").and_then(Json::as_str).unwrap_or("");
    let (vcodec, acodec, ext) = split_mime(mime);
    let protocol = if url.starts_with("https://") { "https" } else { "http" };
    Some(MediaFormat {
        id: item
            .get("itag")
            .and_then(Json::as_u64)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "fmt".into()),
        url: url.to_string(),
        ext,
        width: item.get("width").and_then(Json::as_u64).map(|n| n as u32),
        height: item.get("height").and_then(Json::as_u64).map(|n| n as u32),
        abr: if acodec.as_deref() != Some("none") && vcodec.as_deref() == Some("none") {
            item.get("bitrate").and_then(Json::as_u64)
        } else {
            None
        },
        vbr: if vcodec.as_deref() != Some("none") {
            item.get("bitrate").and_then(Json::as_u64)
        } else {
            None
        },
        tbr: item.get("bitrate").and_then(Json::as_u64),
        filesize: item.get("contentLength").and_then(Json::as_u64),
        vcodec,
        acodec,
        protocol: protocol.into(),
        has_drm: false,
    })
}

fn split_mime(mime: &str) -> (Option<String>, Option<String>, String) {
    let (base, rest) = mime
        .split_once(';')
        .map(|(base, rest)| (base.trim(), rest))
        .unwrap_or((mime.trim(), ""));
    let ext = match base {
        "video/mp4" => "mp4",
        "audio/mp4" => "m4a",
        "audio/webm" | "video/webm" => "webm",
        "video/3gpp" => "3gp",
        _ if base.starts_with("audio/") => "audio",
        _ => "mp4",
    };
    let codecs = rest
        .split("codecs=")
        .nth(1)
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    let mut video = None;
    let mut audio = None;
    if !codecs.is_empty() {
        for codec in codecs.split(',') {
            let codec = codec.trim().trim_matches('"');
            if codec.starts_with("avc")
                || codec.starts_with("vp")
                || codec.starts_with("av01")
                || codec.starts_with("hev")
            {
                video = Some(codec.to_string());
            } else if codec.starts_with("mp4a")
                || codec.starts_with("opus")
                || codec.starts_with("vorbis")
                || codec.starts_with("ac-3")
            {
                audio = Some(codec.to_string());
            }
        }
    }
    if base.starts_with("audio/") {
        video = Some("none".into());
        if audio.is_none() {
            audio = Some("audio".into());
        }
    } else if base.starts_with("video/") {
        if video.is_none() {
            video = Some("video".into());
        }
        if audio.is_none() && codecs.is_empty() {
            audio = Some("none".into());
        }
    }
    (video, audio, ext.into())
}

fn extract_json(html: &str, marker: &str) -> Option<Json> {
    let index = html.find(marker)?;
    let rest = html[index + marker.len()..].trim_start_matches([' ', '\n', '\r', '\t', '=', ':']);
    parse_json(rest).ok()
}

#[allow(dead_code)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get<'a>(&'a self, key: &str) -> Option<&'a Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(text) => Some(text),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) if n.is_finite() && *n >= 0.0 && *n < (u64::MAX as f64) => Some(*n as u64),
            Json::Str(text) => text.parse().ok(),
            _ => None,
        }
    }
}

fn parse_json(input: &str) -> Result<Json> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        index: 0,
        depth: 0,
    };
    parser.value()
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn value(&mut self) -> Result<Json> {
        self.skip();
        if self.depth > 48 {
            return Err(Error::Http("json too deep".into()));
        }
        self.depth += 1;
        let parsed = match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => {
                self.literal(b"true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.literal(b"false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.literal(b"null")?;
                Ok(Json::Null)
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(Error::Http("bad json".into())),
        };
        self.depth -= 1;
        parsed
    }

    fn object(&mut self) -> Result<Json> {
        self.bump(b'{')?;
        let mut pairs = Vec::new();
        loop {
            self.skip();
            if self.peek() == Some(b'}') {
                self.index += 1;
                break;
            }
            let key = self.string()?;
            self.skip();
            self.bump(b':')?;
            let value = self.value()?;
            pairs.push((key, value));
            self.skip();
            match self.peek() {
                Some(b',') => self.index += 1,
                Some(b'}') => {
                    self.index += 1;
                    break;
                }
                _ => return Err(Error::Http("bad json object".into())),
            }
        }
        Ok(Json::Obj(pairs))
    }

    fn array(&mut self) -> Result<Json> {
        self.bump(b'[')?;
        let mut items = Vec::new();
        loop {
            self.skip();
            if self.peek() == Some(b']') {
                self.index += 1;
                break;
            }
            items.push(self.value()?);
            self.skip();
            match self.peek() {
                Some(b',') => self.index += 1,
                Some(b']') => {
                    self.index += 1;
                    break;
                }
                _ => return Err(Error::Http("bad json array".into())),
            }
        }
        Ok(Json::Arr(items))
    }

    fn string(&mut self) -> Result<String> {
        self.skip();
        self.bump(b'"')?;
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            self.index += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.peek().ok_or_else(|| Error::Http("bad json string".into()))?;
                    self.index += 1;
                    match esc {
                        b'"' | b'\\' | b'/' => out.push(esc as char),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut hex = [0u8; 4];
                            for slot in &mut hex {
                                *slot = self.peek().ok_or_else(|| Error::Http("bad json u".into()))?;
                                self.index += 1;
                            }
                            let text = std::str::from_utf8(&hex)
                                .map_err(|_| Error::Http("bad json u".into()))?;
                            let code = u32::from_str_radix(text, 16)
                                .map_err(|_| Error::Http("bad json u".into()))?;
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(Error::Http("bad json escape".into())),
                    }
                }
                _ => out.push(byte as char),
            }
        }
        Err(Error::Http("unterminated json string".into()))
    }

    fn number(&mut self) -> Result<Json> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        )
        {
            self.index += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index])
            .map_err(|_| Error::Http("bad json number".into()))?;
        let number = text
            .parse::<f64>()
            .map_err(|_| Error::Http("bad json number".into()))?;
        Ok(Json::Num(number))
    }

    fn literal(&mut self, expected: &[u8]) -> Result<()> {
        if self.bytes[self.index..].starts_with(expected) {
            self.index += expected.len();
            Ok(())
        } else {
            Err(Error::Http("bad json literal".into()))
        }
    }

    fn bump(&mut self, expected: u8) -> Result<()> {
        self.skip();
        if self.peek() == Some(expected) {
            self.index += 1;
            Ok(())
        } else {
            Err(Error::Http("bad json".into()))
        }
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.index += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }
}

fn write_stored_zip(paths: &[PathBuf], dest: &Path) -> Result<()> {
    let mut out = File::create(dest)?;
    let mut central = Vec::new();
    let mut count = 0u16;
    let mut offset = 0u32;
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .as_bytes();
        if name.len() > u16::MAX as usize {
            return Err(Error::Http("zip name too long".into()));
        }
        let bytes = std::fs::read(path)?;
        let crc = crc32(&bytes);
        let size = u32::try_from(bytes.len()).map_err(|_| Error::TooLarge {
            limit: u32::MAX as u64,
            got: bytes.len() as u64,
        })?;
        let name_len = name.len() as u16;
        write_u32(&mut out, 0x0403_4b50)?;
        write_u16(&mut out, 20)?;
        write_u16(&mut out, 0)?;
        write_u16(&mut out, 0)?;
        write_u16(&mut out, 0)?;
        write_u16(&mut out, 0)?;
        write_u32(&mut out, crc)?;
        write_u32(&mut out, size)?;
        write_u32(&mut out, size)?;
        write_u16(&mut out, name_len)?;
        write_u16(&mut out, 0)?;
        out.write_all(name)?;
        out.write_all(&bytes)?;
        let local = 30 + name.len() as u32 + size;
        write_u32(&mut central, 0x0201_4b50)?;
        write_u16(&mut central, 20)?;
        write_u16(&mut central, 20)?;
        write_u16(&mut central, 0)?;
        write_u16(&mut central, 0)?;
        write_u16(&mut central, 0)?;
        write_u16(&mut central, 0)?;
        write_u32(&mut central, crc)?;
        write_u32(&mut central, size)?;
        write_u32(&mut central, size)?;
        write_u16(&mut central, name_len)?;
        write_u16(&mut central, 0)?;
        write_u16(&mut central, 0)?;
        write_u16(&mut central, 0)?;
        write_u16(&mut central, 0)?;
        write_u32(&mut central, 0)?;
        write_u32(&mut central, offset)?;
        central.extend_from_slice(name);
        offset = offset.saturating_add(local);
        count += 1;
    }
    let central_start = offset;
    out.write_all(&central)?;
    write_u32(&mut out, 0x0605_4b50)?;
    write_u16(&mut out, 0)?;
    write_u16(&mut out, 0)?;
    write_u16(&mut out, count)?;
    write_u16(&mut out, count)?;
    write_u32(&mut out, central.len() as u32)?;
    write_u32(&mut out, central_start)?;
    write_u16(&mut out, 0)?;
    Ok(())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn write_u16(out: &mut impl Write, value: u16) -> Result<()> {
    out.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u32(out: &mut impl Write, value: u32) -> Result<()> {
    out.write_all(&value.to_le_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{self, Resp};
    use std::time::Duration;

    fn cfg() -> GetCfg {
        GetCfg {
            timeout: Duration::from_secs(2),
            max_redirects: 4,
            max_bytes: None,
            check_ssrf: true,
            permit_loopback: true,
        }
    }

    #[test]
    fn direct_mp4_and_gallery_and_watch_page() {
        let video = b"\0\0\0\x18ftypisom\0\0\0\0isom".to_vec();
        let jpeg = b"\xff\xd8\xff\xe0jpeg-bytes".to_vec();
        let png = b"\x89PNG\r\n\x1a\npng-bytes".to_vec();
        let video_b = video.clone();
        let jpeg_b = jpeg.clone();
        let png_b = png.clone();
        let server = harness::serve(move |req| {
            let path = req.target.split('?').next().unwrap_or("/");
            if path == "/clip.mp4" {
                if req.method == "HEAD" {
                    return Resp {
                        status: 200,
                        headers: vec![
                            ("Content-Type".into(), "video/mp4".into()),
                            ("Content-Length".into(), video_b.len().to_string()),
                            ("Accept-Ranges".into(), "bytes".into()),
                        ],
                        body: Vec::new(),
                        chunked: false,
                    };
                }
                return Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "video/mp4".into())],
                    body: video_b.clone(),
                    chunked: false,
                };
            }
            if path == "/a.jpg" {
                return image_resp(&req.method, &jpeg_b, "image/jpeg");
            }
            if path == "/b.png" {
                return image_resp(&req.method, &png_b, "image/png");
            }
            if path == "/page" {
                let html = format!(
                    "<html><title>Album</title><img src=\"/a.jpg\"><img src=\"/b.png\"></html>"
                );
                return Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/html".into())],
                    body: html.into_bytes(),
                    chunked: false,
                };
            }
            if path == "/watch" {
                let html = "<html><meta property=\"og:title\" content=\"Clip\"><meta property=\"og:video\" content=\"/clip.mp4\"></html>";
                return Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/html".into())],
                    body: html.as_bytes().to_vec(),
                    chunked: false,
                };
            }
            Resp {
                status: 404,
                headers: vec![],
                body: Vec::new(),
                chunked: false,
            }
        });
        let root = std::env::temp_dir().join(format!("export-fetch-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mp4_dir = root.join("mp4");
        let job = fetch_job(&server.url("/clip.mp4"), "mp4", &cfg(), &mp4_dir).unwrap();
        assert_eq!(job.kind, JobKind::Video);
        let saved = std::fs::read(&job.paths[0]).unwrap();
        assert_eq!(saved, video);

        let notes = fetch_job(&server.url("/clip.mp4"), "notes", &cfg(), &root.join("notes")).unwrap();
        assert_eq!(notes.kind, JobKind::Audio);
        assert_eq!(std::fs::read(&notes.paths[0]).unwrap(), video);

        let frames = fetch_job(&server.url("/clip.mp4"), "frames", &cfg(), &root.join("frames")).unwrap();
        assert_eq!(frames.kind, JobKind::Video);

        let images = fetch_job(&server.url("/page"), "images", &cfg(), &root.join("img")).unwrap();
        assert_eq!(images.paths.len(), 1);
        let zip = std::fs::read(&images.paths[0]).unwrap();
        assert!(zip.windows(jpeg.len()).any(|w| w == jpeg.as_slice()));
        assert!(zip.windows(png.len()).any(|w| w == png.as_slice()));
        assert_eq!(images.title, "Album");

        let watch = fetch_job(&server.url("/watch"), "mp4", &cfg(), &root.join("watch")).unwrap();
        assert_eq!(watch.title, "Clip");
        assert_eq!(std::fs::read(&watch.paths[0]).unwrap(), video);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn image_resp(method: &str, body: &[u8], ct: &str) -> Resp {
        if method == "HEAD" {
            Resp {
                status: 200,
                headers: vec![
                    ("Content-Type".into(), ct.into()),
                    ("Content-Length".into(), body.len().to_string()),
                ],
                body: Vec::new(),
                chunked: false,
            }
        } else {
            Resp {
                status: 200,
                headers: vec![("Content-Type".into(), ct.into())],
                body: body.to_vec(),
                chunked: false,
            }
        }
    }

    #[test]
    fn youtube_keeps_progressive_urls_and_skips_cipher_and_drm() {
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        assert_eq!(
            video_id("https://youtu.be/dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
        let html = r#"<html><script>var ytInitialPlayerResponse = {"videoDetails":{"title":"Demo"},"streamingData":{"formats":[
            {"itag":18,"url":"http://cdn/p.mp4","mimeType":"video/mp4; codecs=\"avc1.4d401e, mp4a.40.2\"","width":640,"height":360,"bitrate":500000,"contentLength":"1000"},
            {"itag":22,"signatureCipher":"s=AA&sp=sig&url=http%3A%2F%2Fcdn%2Fx","mimeType":"video/mp4","height":720}
        ],"adaptiveFormats":[
            {"itag":140,"url":"http://cdn/a.m4a","mimeType":"audio/mp4; codecs=\"mp4a.40.2\"","bitrate":128000,"contentLength":"500"},
            {"itag":251,"url":"http://cdn/a.webm","mimeType":"audio/webm; codecs=\"opus\"","bitrate":160000,"contentLength":"400"}
        ]}};</script></html>"#;
        let resolved = YoutubeAdapter::from_html(html).unwrap();
        assert_eq!(resolved.title, "Demo");
        assert_eq!(resolved.formats.len(), 3);
        assert!(resolved.formats.iter().all(|fmt| !fmt.url.contains("signature")));
        let audio = select::select(&resolved.formats, Want::Audio, Some(450)).unwrap();
        assert_eq!(audio.primary.id, "251");
        let video = select::select(&resolved.formats, Want::Video, None).unwrap();
        assert_eq!(video.primary.id, "18");
        let cipher_only = r#"ytInitialPlayerResponse = {"streamingData":{"formats":[{"itag":22,"signatureCipher":"s=AA"}]}}"#;
        let err = YoutubeAdapter::from_html(cipher_only).unwrap_err();
        assert!(matches!(err, Error::NotDirectlyFetchable(_)), "{err}");
        let drm = r#"ytInitialPlayerResponse = {"playabilityStatus":{"status":"UNPLAYABLE","reason":"DRM protected"}}"#;
        let err = YoutubeAdapter::from_html(drm).unwrap_err();
        assert!(err.to_string().contains("drm"), "{err}");
        let err = YoutubeAdapter
            .resolve("https://www.youtube.com/watch?v=dQw4w9WgXcQ", &cfg())
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{err}");
        assert!(!DirectAdapter.name().is_empty());
    }

    #[test]
    fn sniff_does_not_trust_the_extension() {
        assert_eq!(sniff(b"\xff\xd8\xff\xe0"), Some("jpeg"));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(sniff(b"\0\0\0\x18ftypisom"), Some("mp4"));
    }
}
