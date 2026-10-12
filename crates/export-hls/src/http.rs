use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::error::Error;
use crate::model::{ByteRange, MediaPlaylist};
use crate::parse::parse_media_playlist;
use crate::policy::ConnectPolicy;
use crate::urlcheck::{check_media_uri, parse_http_url, HttpUrl};

const HEADER_CAP: usize = 64 * 1024;
const USER_AGENT: &str = "export-hls/0.1";

/// Cleartext HTTP/1.1 client. No cookies, no TLS, no browser impersonation.
#[derive(Debug, Clone)]
pub struct Fetcher {
    pub policy: ConnectPolicy,
    pub timeout: Duration,
    pub max_redirects: u8,
    pub max_body_bytes: usize,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self {
            policy: ConnectPolicy::default(),
            timeout: Duration::from_secs(10),
            max_redirects: 5,
            max_body_bytes: 8 * 1024 * 1024,
        }
    }
}

impl Fetcher {
    pub fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
        self.fetch(url, None)
    }

    pub fn get_range(&self, url: &str, range: ByteRange) -> Result<Vec<u8>, Error> {
        self.fetch(url, Some(range))
    }

    pub fn get_playlist(&self, url: &str) -> Result<MediaPlaylist, Error> {
        let body = self.get(url)?;
        let text =
            std::str::from_utf8(&body).map_err(|_| Error::Http("playlist is not utf-8".into()))?;
        parse_media_playlist(text)
    }

    /// Fetch one segment named by `playlist` with this cleartext client.
    ///
    /// The segment URI is requested with [`Fetcher::get`]. `https` URIs are
    /// unsupported. This method does not run yt-dlp or any other program.
    pub fn get_segment(&self, playlist: &MediaPlaylist, index: usize) -> Result<Vec<u8>, Error> {
        let segment = playlist.segments.get(index).ok_or(Error::BadTag)?;
        self.get(&segment.uri)
    }

    /// Fetch every segment named by one media playlist with this cleartext client.
    ///
    /// Each URI is requested with [`Fetcher::get_segment`], in playlist order.
    /// `https` URIs are unsupported. This method does not run yt-dlp or any
    /// other program. An empty segment list returns [`Error::BadTag`].
    pub fn get_segments(&self, playlist: &MediaPlaylist) -> Result<Vec<Vec<u8>>, Error> {
        if playlist.segments.is_empty() {
            return Err(Error::BadTag);
        }
        let mut bodies = Vec::with_capacity(playlist.segments.len());
        for index in 0..playlist.segments.len() {
            bodies.push(self.get_segment(playlist, index)?);
        }
        Ok(bodies)
    }

    fn fetch(&self, url: &str, range: Option<ByteRange>) -> Result<Vec<u8>, Error> {
        let timeout = self.timeout;
        let max_body = self.max_body_bytes;
        follow_hops(url, self.policy, self.max_redirects, |hop| {
            tcp_exchange(hop, timeout, max_body, range)
        })
    }
}

/// One checked hop: the address was classified before connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub addr: SocketAddr,
    pub url: HttpUrl,
}

/// Status, optional redirect location, and body. Cookie responses never
/// become a [`HttpMessage`]; they return [`Error::Unsupported`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpMessage {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// Resolve `url` and refuse loopback or link-local before any connection.
///
/// Callers run this on the original URL and again on every redirect target.
pub fn check_hop(url: &str, policy: ConnectPolicy) -> Result<Hop, Error> {
    let parsed = parse_http_url(url)?;
    let host = parsed.host.clone();
    let addrs = resolve_addrs(&host, parsed.port, policy)?;
    let addr = *addrs
        .first()
        .ok_or_else(|| Error::Http("no addresses".into()))?;
    Ok(Hop { addr, url: parsed })
}

/// Join a `Location` value onto `base`. Absolute `https`, userinfo, cookies,
/// and credential query keys return [`Error::Unsupported`].
pub fn resolve_redirect(base: &HttpUrl, location: &str) -> Result<String, Error> {
    let location = location.trim();
    if location.is_empty() {
        return Err(Error::Http("empty location".into()));
    }
    if location.starts_with("//") {
        return Err(Error::Unsupported);
    }
    if scheme_present(location) {
        check_media_uri(location)?;
        let absolute = parse_http_url(location)?;
        return Ok(url_string(&absolute));
    }
    let path = if let Some(stripped) = location.strip_prefix('/') {
        let owned = format!("/{stripped}");
        strip_fragment_owned(&owned)
    } else {
        let base_path = base.path_and_query.split('?').next().unwrap_or("/");
        let dir = match base_path.rfind('/') {
            Some(index) => &base_path[..=index],
            None => "/",
        };
        strip_fragment_owned(&format!("{dir}{location}"))
    };
    let joined = if base.port == 80 {
        format!("http://{host}{path}", host = base.host)
    } else {
        format!(
            "http://{host}:{port}{path}",
            host = base.host,
            port = base.port
        )
    };
    check_media_uri(&joined)?;
    Ok(joined)
}

/// Write one cleartext `GET`. The request carries no `Cookie` and no
/// `Authorization` header. `User-Agent` is `export-hls/0.1`.
pub fn write_http_get(
    writer: &mut impl Write,
    url: &HttpUrl,
    range: Option<ByteRange>,
) -> Result<(), Error> {
    let mut request = String::new();
    request.push_str("GET ");
    request.push_str(&url.path_and_query);
    request.push_str(" HTTP/1.1\r\nHost: ");
    request.push_str(&url.host_header());
    request.push_str("\r\nUser-Agent: ");
    request.push_str(USER_AGENT);
    request.push_str("\r\nAccept: */*\r\n");
    if let Some(range) = range {
        let end = range
            .end()
            .ok_or(Error::BadTag)?
            .checked_sub(1)
            .ok_or(Error::BadTag)?;
        request.push_str(&format!("Range: bytes={}-{end}\r\n", range.offset));
    }
    request.push_str("Connection: close\r\n\r\n");
    writer.write_all(request.as_bytes()).map_err(io_err)?;
    Ok(())
}

/// Read one HTTP/1.0 or HTTP/1.1 response. `Set-Cookie`, `Cookie`, and
/// `Authorization` headers return [`Error::Unsupported`].
pub fn read_http_message(reader: &mut impl Read, max_body: usize) -> Result<HttpMessage, Error> {
    let (header, leftover) = read_header_block(reader)?;
    let text =
        std::str::from_utf8(&header).map_err(|_| Error::Http("headers are not utf-8".into()))?;
    let parsed = parse_header_block(text)?;
    let prefix = std::io::Cursor::new(leftover);
    let mut chained = prefix.chain(reader);
    let body = read_body(&mut chained, &parsed, max_body)?;
    Ok(HttpMessage {
        status: parsed.status,
        location: parsed.location,
        body,
    })
}

fn follow_hops(
    start: &str,
    policy: ConnectPolicy,
    max_redirects: u8,
    mut exchange: impl FnMut(&Hop) -> Result<HttpMessage, Error>,
) -> Result<Vec<u8>, Error> {
    let mut current = start.to_string();
    let mut redirects = 0u8;
    loop {
        let hop = check_hop(&current, policy)?;
        let message = exchange(&hop)?;
        if is_redirect(message.status) {
            if redirects >= max_redirects {
                return Err(Error::Http("too many redirects".into()));
            }
            let location = message
                .location
                .ok_or_else(|| Error::Http("redirect missing location".into()))?;
            current = resolve_redirect(&hop.url, &location)?;
            redirects += 1;
            continue;
        }
        if !(200..300).contains(&message.status) {
            return Err(Error::Http(format!("status {}", message.status)));
        }
        return Ok(message.body);
    }
}

fn tcp_exchange(
    hop: &Hop,
    timeout: Duration,
    max_body: usize,
    range: Option<ByteRange>,
) -> Result<HttpMessage, Error> {
    let mut stream = TcpStream::connect_timeout(&hop.addr, timeout).map_err(io_err)?;
    stream.set_read_timeout(Some(timeout)).map_err(io_err)?;
    stream.set_write_timeout(Some(timeout)).map_err(io_err)?;
    write_http_get(&mut stream, &hop.url, range)?;
    stream.flush().map_err(io_err)?;
    read_http_message(&mut stream, max_body)
}

fn resolve_addrs(host: &str, port: u16, policy: ConnectPolicy) -> Result<Vec<SocketAddr>, Error> {
    if let Some(ip) = ip_literal(host) {
        policy.check_ip(ip)?;
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let looked_up = (host, port)
        .to_socket_addrs()
        .map_err(|err| Error::Http(err.to_string()))?;
    let mut addrs = Vec::new();
    for addr in looked_up {
        policy.check_ip(addr.ip())?;
        addrs.push(addr);
    }
    if addrs.is_empty() {
        return Err(Error::Http("no addresses".into()));
    }
    Ok(addrs)
}

fn ip_literal(host: &str) -> Option<IpAddr> {
    let bare = if host.starts_with('[') && host.ends_with(']') && host.len() > 2 {
        &host[1..host.len() - 1]
    } else {
        host
    };
    bare.parse().ok()
}

fn url_string(url: &HttpUrl) -> String {
    if url.port == 80 {
        format!(
            "http://{host}{path}",
            host = url.host,
            path = url.path_and_query
        )
    } else {
        format!(
            "http://{host}:{port}{path}",
            host = url.host,
            port = url.port,
            path = url.path_and_query
        )
    }
}

fn scheme_present(uri: &str) -> bool {
    let bytes = uri.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let mut index = 1;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'-' || byte == b'.' {
            index += 1;
        } else {
            break;
        }
    }
    uri[index..].starts_with("://")
}

fn strip_fragment_owned(value: &str) -> String {
    value.split('#').next().unwrap_or(value).to_string()
}

struct ParsedHeaders {
    status: u16,
    location: Option<String>,
    content_length: Option<usize>,
    chunked: bool,
}

fn parse_header_block(text: &str) -> Result<ParsedHeaders, Error> {
    let text = text
        .strip_suffix("\r\n\r\n")
        .ok_or_else(|| Error::Http("bad headers".into()))?;
    let mut lines = text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| Error::Http("missing status".into()))?;
    let mut parsed = ParsedHeaders {
        status: parse_status(status_line)?,
        location: None,
        content_length: None,
        chunked: false,
    };
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(Error::Http("folded header".into()));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| Error::Http("bad header".into()))?;
        let value = value.trim();
        if is_forbidden_header(name) {
            return Err(Error::Unsupported);
        }
        if name.eq_ignore_ascii_case("location") {
            if parsed.location.is_some() {
                return Err(Error::Http("multiple location headers".into()));
            }
            parsed.location = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            if parsed.content_length.is_some() {
                return Err(Error::Http("multiple content-length headers".into()));
            }
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(Error::Http("bad content-length".into()));
            }
            let len: usize = value
                .parse()
                .map_err(|_| Error::Http("bad content-length".into()))?;
            parsed.content_length = Some(len);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            let chunked = value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"));
            if chunked {
                parsed.chunked = true;
            } else if !value.eq_ignore_ascii_case("identity") {
                return Err(Error::Http("unsupported transfer-encoding".into()));
            }
        }
    }
    if parsed.chunked && parsed.content_length.is_some() {
        return Err(Error::Http(
            "chunked response also has content-length".into(),
        ));
    }
    Ok(parsed)
}

fn is_forbidden_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("set-cookie")
        || name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
}

fn parse_status(line: &str) -> Result<u16, Error> {
    let mut parts = line.split(' ');
    let version = parts
        .next()
        .ok_or_else(|| Error::Http("bad status".into()))?;
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(Error::Http("unsupported http version".into()));
    }
    let code = parts
        .next()
        .ok_or_else(|| Error::Http("bad status".into()))?;
    if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::Http("bad status".into()));
    }
    code.parse().map_err(|_| Error::Http("bad status".into()))
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn read_body(
    reader: &mut impl Read,
    headers: &ParsedHeaders,
    max_body: usize,
) -> Result<Vec<u8>, Error> {
    if headers.chunked {
        read_chunked(reader, max_body)
    } else if let Some(len) = headers.content_length {
        read_exact_limited(reader, len, max_body)
    } else {
        read_to_end_limited(reader, max_body)
    }
}

fn read_exact_limited(
    reader: &mut impl Read,
    len: usize,
    max_body: usize,
) -> Result<Vec<u8>, Error> {
    if len > max_body {
        return Err(Error::Http("body exceeds limit".into()));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).map_err(io_err)?;
    Ok(body)
}

fn read_to_end_limited(reader: &mut impl Read, max_body: usize) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let read = reader.read(&mut tmp).map_err(io_err)?;
        if read == 0 {
            return Ok(body);
        }
        if body.len() + read > max_body {
            return Err(Error::Http("body exceeds limit".into()));
        }
        body.extend_from_slice(&tmp[..read]);
    }
}

fn read_chunked(reader: &mut impl Read, max_body: usize) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    loop {
        let line = read_crlf_line(reader)?;
        let size_text = line.split(';').next().unwrap_or("").trim();
        if size_text.is_empty() || !size_text.bytes().all(is_hex) {
            return Err(Error::Http("bad chunk size".into()));
        }
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| Error::Http("bad chunk size".into()))?;
        if size == 0 {
            loop {
                let trailer = read_crlf_line(reader)?;
                if trailer.is_empty() {
                    break;
                }
                let name = trailer.split(':').next().unwrap_or("");
                if is_forbidden_header(name.trim()) {
                    return Err(Error::Unsupported);
                }
            }
            return Ok(body);
        }
        if body.len().saturating_add(size) > max_body {
            return Err(Error::Http("body exceeds limit".into()));
        }
        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk).map_err(io_err)?;
        body.extend_from_slice(&chunk);
        let ending = read_exact_n(reader, 2)?;
        if ending.as_slice() != b"\r\n" {
            return Err(Error::Http("chunk missing crlf".into()));
        }
    }
}

fn read_crlf_line(reader: &mut impl Read) -> Result<String, Error> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if buf.len() >= 8192 {
            return Err(Error::Http("line too long".into()));
        }
        let read = reader.read(&mut byte).map_err(io_err)?;
        if read == 0 {
            return Err(Error::Http("connection closed".into()));
        }
        if byte[0] == b'\n' {
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            let text =
                String::from_utf8(buf).map_err(|_| Error::Http("line is not utf-8".into()))?;
            return Ok(text);
        }
        buf.push(byte[0]);
    }
}

fn read_exact_n(reader: &mut impl Read, len: usize) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(io_err)?;
    Ok(buf)
}

fn read_header_block(reader: &mut impl Read) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let read = reader.read(&mut tmp).map_err(io_err)?;
        if read == 0 {
            return Err(Error::Http("connection closed before headers".into()));
        }
        buf.extend_from_slice(&tmp[..read]);
        if let Some(end) = find_subsequence(&buf, b"\r\n\r\n") {
            let leftover = buf[end + 4..].to_vec();
            buf.truncate(end + 4);
            return Ok((buf, leftover));
        }
        if buf.len() > HEADER_CAP {
            return Err(Error::Http("headers too large".into()));
        }
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// `u8::is_ascii_hexdigit` stabilized after rust-version 1.74.
#[allow(clippy::manual_is_ascii_check)]
fn is_hex(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')
}

fn io_err(err: io::Error) -> Error {
    Error::Http(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ByteRange;
    use std::io::Cursor;

    #[test]
    fn request_has_no_cookie_and_no_browser_user_agent() {
        let url = parse_http_url("http://1.2.3.4/a.m3u8").unwrap();
        let mut buf = Vec::new();
        write_http_get(
            &mut buf,
            &url,
            Some(ByteRange {
                offset: 10,
                length: 5,
            }),
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.starts_with("GET /a.m3u8 HTTP/1.1\r\n"));
        assert!(text.contains("Range: bytes=10-14\r\n"));
        assert!(text.contains("User-Agent: export-hls/0.1\r\n"));
        let lower = text.to_ascii_lowercase();
        assert!(!lower.contains("cookie"));
        assert!(!lower.contains("authorization"));
        assert!(!text.contains("Mozilla"));
        assert!(!text.contains("Chrome"));
    }

    #[test]
    fn set_cookie_response_is_unsupported() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nSet-Cookie: a=b\r\n\r\nok";
        let err = read_http_message(&mut Cursor::new(&raw[..]), 1024).unwrap_err();
        assert_eq!(err, Error::Unsupported);
    }

    #[test]
    fn content_length_and_chunked_bodies() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhelloEXTRA";
        let message = read_http_message(&mut Cursor::new(&raw[..]), 1024).unwrap();
        assert_eq!(message.status, 200);
        assert_eq!(message.body, b"hello");

        let chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let message = read_http_message(&mut Cursor::new(&chunked[..]), 1024).unwrap();
        assert_eq!(message.body, b"hello");
    }

    #[test]
    fn preflight_refuses_https_userinfo_cookies_loopback_and_link_local() {
        let deny = ConnectPolicy::default();
        assert_eq!(
            check_hop("https://cdn.example/a.m3u8", deny).unwrap_err(),
            Error::Unsupported
        );
        assert_eq!(
            check_hop("http://user:pass@1.2.3.4/a.m3u8", deny).unwrap_err(),
            Error::Unsupported
        );
        assert_eq!(
            check_hop("http://1.2.3.4/a.m3u8?cookie=1", deny).unwrap_err(),
            Error::Unsupported
        );
        assert_eq!(
            check_hop("http://127.0.0.1/a.m3u8", deny).unwrap_err(),
            Error::Blocked
        );
        assert_eq!(
            check_hop("http://[::1]/a.m3u8", deny).unwrap_err(),
            Error::Blocked
        );
        assert_eq!(
            check_hop("http://169.254.1.1/a.m3u8", ConnectPolicy::new(true)).unwrap_err(),
            Error::Blocked
        );
        let hop = check_hop("http://127.0.0.1:9/a.m3u8", ConnectPolicy::new(true)).unwrap();
        assert_eq!(hop.addr.port(), 9);
        assert!(hop.addr.ip().is_loopback());
        let public = check_hop("http://1.2.3.4/a.m3u8", deny).unwrap();
        assert_eq!(public.addr.port(), 80);
    }

    #[test]
    fn redirect_targets_are_checked_again() {
        let next = resolve_redirect(
            &parse_http_url("http://1.2.3.4/dir/a.m3u8").unwrap(),
            "/b.m3u8",
        )
        .unwrap();
        assert_eq!(next, "http://1.2.3.4/b.m3u8");

        let https = resolve_redirect(
            &parse_http_url("http://1.2.3.4/a.m3u8").unwrap(),
            "https://1.2.3.4/a.m3u8",
        );
        assert_eq!(https, Err(Error::Unsupported));

        let mut hosts = Vec::new();
        let err = follow_hops(
            "http://1.2.3.4/a.m3u8",
            ConnectPolicy::default(),
            5,
            |hop| {
                hosts.push(hop.url.host.clone());
                Ok(HttpMessage {
                    status: 302,
                    location: Some("http://127.0.0.1/a.m3u8".into()),
                    body: Vec::new(),
                })
            },
        );
        assert_eq!(err, Err(Error::Blocked));
        assert_eq!(hosts, vec!["1.2.3.4".to_string()]);
    }

    #[test]
    fn fetcher_rejects_cleartext_violations_before_connect() {
        let fetcher = Fetcher::default();
        assert_eq!(
            fetcher.get("https://cdn.example/a.m3u8"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            fetcher.get("http://user:secret@1.2.3.4/a.m3u8"),
            Err(Error::Unsupported)
        );
        assert_eq!(fetcher.get("http://127.0.0.1/a.m3u8"), Err(Error::Blocked));
    }

    #[test]
    fn one_named_segment_stays_on_the_cleartext_client() {
        assert_eq!(
            parse_media_playlist("#EXTM3U\n#EXTINF:1,\nhttps://cdn.example/seg.ts\n"),
            Err(Error::Unsupported)
        );
        let playlist = parse_media_playlist("#EXTM3U\n#EXTINF:1,\nseg.ts\n").unwrap();
        assert_eq!(playlist.segments.len(), 1);
        assert_eq!(
            Fetcher::default().get_segment(&playlist, 0),
            Err(Error::BadTag)
        );
        assert_eq!(
            Fetcher::default().get_segment(&playlist, 1),
            Err(Error::BadTag)
        );
    }

    #[test]
    fn two_named_segments_stay_on_the_cleartext_client() {
        assert_eq!(
            parse_media_playlist(
                "#EXTM3U\n#EXTINF:1,\nhttp://cdn.example/a.ts\n#EXTINF:1,\nhttps://cdn.example/b.ts\n"
            ),
            Err(Error::Unsupported)
        );
        let playlist =
            parse_media_playlist("#EXTM3U\n#EXTINF:1,\na.ts\n#EXTINF:1,\nb.ts\n").unwrap();
        assert_eq!(playlist.segments.len(), 2);
        assert_eq!(
            Fetcher::default().get_segments(&playlist),
            Err(Error::BadTag)
        );
        let empty = parse_media_playlist("#EXTM3U\n#EXT-X-ENDLIST\n").unwrap();
        assert!(empty.segments.is_empty());
        assert_eq!(Fetcher::default().get_segments(&empty), Err(Error::BadTag));
    }
}
