//! Cleartext HTTP/1.1 over `std::net`. No TLS, no browser impersonation, no cookies.

use crate::error::{Error, Result};
use crate::policy::{self, Url};
use crate::range::{ByteRange, RangeSpec};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

pub const READ_BUF: usize = 64 * 1024;
pub const USER_AGENT: &str = "export-fetch/0.1";

#[derive(Clone, Debug)]
pub struct GetCfg {
    pub timeout: Duration,
    pub max_redirects: usize,
    pub max_bytes: Option<u64>,
    pub check_ssrf: bool,
    /// Allows 127.0.0.0/8 and ::1 while still refusing other private ranges.
    /// The default job config leaves this off.
    pub permit_loopback: bool,
}

impl Default for GetCfg {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_redirects: 8,
            max_bytes: None,
            check_ssrf: true,
            permit_loopback: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Head {
    pub status: u16,
    pub final_url: String,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub accept_ranges: bool,
    /// start, end (exclusive), total
    pub content_range: Option<(u64, u64, Option<u64>)>,
}

pub enum Fetched {
    Whole(Vec<u8>),
    Part(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct DownloadOutcome {
    pub bytes_written: u64,
    pub status: u16,
    pub final_url: String,
    pub content_type: Option<String>,
    pub restarted: bool,
    pub peak_write: u64,
    pub read_buffer: u64,
}

enum Kind {
    Empty,
    Eof,
    Length,
    Chunk,
}

struct Response {
    head: Head,
    tcp: TcpStream,
    buf: Vec<u8>,
    pos: usize,
    kind: Kind,
    left: u64,
    chunk_need_line: bool,
    chunk_done: bool,
    /// Body ended on a content-length and the server did not ask to close.
    persist_ok: bool,
}

/// One cleartext socket reused across GETs to the same host and port.
///
/// `download_to` still opens a new connection per call.
pub struct ClearConn {
    cfg: GetCfg,
    idle: Option<Idle>,
    dials: u64,
}

struct Idle {
    tcp: TcpStream,
    host: String,
    port: u16,
}

impl ClearConn {
    pub fn new(cfg: GetCfg) -> Self {
        Self {
            cfg,
            idle: None,
            dials: 0,
        }
    }

    pub fn connections(&self) -> u64 {
        self.dials
    }

    pub fn download_to<W: Write>(
        &mut self,
        url: &str,
        writer: &mut W,
    ) -> Result<DownloadOutcome> {
        let current = policy::parse_url(url)?;
        if current.scheme != "http" {
            return Err(Error::Unsupported(
                "cleartext http only; this crate has no TLS".into(),
            ));
        }
        enforce(&current, &self.cfg)?;
        let tcp = match self.idle.take() {
            Some(idle) if idle.host == current.host && idle.port == current.port => idle.tcp,
            _ => {
                self.dials += 1;
                dial(&current, &self.cfg)?
            }
        };
        let mut resp = exchange(tcp, &current, None, true)?;
        if resp.head.status != 200 && resp.head.status != 206 {
            return Err(Error::Http(format!("status {}", resp.head.status)));
        }
        if let (Some(max), Some(len)) = (self.cfg.max_bytes, resp.head.content_length) {
            if len > max {
                return Err(Error::TooLarge { limit: max, got: len });
            }
        }
        let (bytes_written, peak_write) = copy_body(&mut resp, self.cfg.max_bytes, writer)?;
        let status = resp.head.status;
        let final_url = resp.head.final_url.clone();
        let content_type = resp.head.content_type.clone();
        if let Some(tcp) = resp.into_idle() {
            self.idle = Some(Idle {
                tcp,
                host: current.host,
                port: current.port,
            });
        }
        Ok(DownloadOutcome {
            bytes_written,
            status,
            final_url,
            content_type,
            restarted: false,
            peak_write,
            read_buffer: READ_BUF as u64,
        })
    }
}

pub fn head(cfg: &GetCfg, url: &str) -> Result<Head> {
    Ok(open(cfg, "HEAD", url, None)?.head)
}

pub fn get_capped(cfg: &GetCfg, url: &str, cap: u64) -> Result<(Head, Vec<u8>)> {
    let mut resp = open(cfg, "GET", url, None)?;
    let head = resp.head.clone();
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let next = out.len() as u64 + n as u64;
        if next > cap {
            return Err(Error::TooLarge { limit: cap, got: next });
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok((head, out))
}

pub fn download_to<W: Write>(
    cfg: &GetCfg,
    url: &str,
    range: Option<RangeSpec>,
    writer: &mut W,
) -> Result<DownloadOutcome> {
    let mut resp = open(cfg, "GET", url, range)?;
    if resp.head.status != 200 && resp.head.status != 206 {
        return Err(Error::Http(format!("status {}", resp.head.status)));
    }
    if let (Some(max), Some(len)) = (cfg.max_bytes, resp.head.content_length) {
        if len > max {
            return Err(Error::TooLarge { limit: max, got: len });
        }
    }
    let (bytes_written, peak_write) = copy_body(&mut resp, cfg.max_bytes, writer)?;
    Ok(DownloadOutcome {
        bytes_written,
        status: resp.head.status,
        final_url: resp.head.final_url,
        content_type: resp.head.content_type,
        restarted: false,
        peak_write,
        read_buffer: READ_BUF as u64,
    })
}

pub fn download_file(cfg: &GetCfg, url: &str, dest: &Path) -> Result<DownloadOutcome> {
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let part = part_path(dest);
    let existing = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let range = if existing > 0 {
        Some(RangeSpec::From(existing))
    } else {
        None
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false) // a resumed .part keeps its bytes; a 200 restart truncates below
        .write(true)
        .read(true)
        .open(&part)?;
    if existing > 0 {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::End(0))?;
    }
    let opened = open(cfg, "GET", url, range);
    let mut resp = match opened {
        Ok(resp) => resp,
        Err(err) => {
            if matches!(err, Error::Ssrf(_) | Error::TooLarge { .. }) {
                let _ = std::fs::remove_file(&part);
            }
            return Err(err);
        }
    };
    let mut restarted = false;
    if existing > 0 && resp.head.status == 200 {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(0))?;
        file.set_len(0)?;
        restarted = true;
    } else if existing > 0 && resp.head.status == 206 {
        if let Some((start, _, _)) = resp.head.content_range {
            if start != existing {
                return Err(Error::Http(
                    "content-range does not match the partial file".into(),
                ));
            }
        }
    } else if resp.head.status != 200 && resp.head.status != 206 {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Http(format!("status {}", resp.head.status)));
    }
    if let (Some(max), Some(len)) = (cfg.max_bytes, resp.head.content_length) {
        let already = if restarted { 0 } else { existing };
        if already.saturating_add(len) > max && resp.head.status == 206 {
            let _ = std::fs::remove_file(&part);
            return Err(Error::TooLarge {
                limit: max,
                got: already.saturating_add(len),
            });
        }
        if resp.head.status == 200 && len > max {
            let _ = std::fs::remove_file(&part);
            return Err(Error::TooLarge { limit: max, got: len });
        }
    }
    let copied = copy_body(&mut resp, cfg.max_bytes, &mut file);
    let (bytes_written, peak_write) = match copied {
        Ok(pair) => pair,
        Err(err) => {
            if matches!(err, Error::TooLarge { .. }) {
                let _ = std::fs::remove_file(&part);
            }
            return Err(err);
        }
    };
    file.flush()?;
    std::fs::rename(&part, dest)?;
    Ok(DownloadOutcome {
        bytes_written,
        status: resp.head.status,
        final_url: resp.head.final_url,
        content_type: resp.head.content_type,
        restarted,
        peak_write,
        read_buffer: READ_BUF as u64,
    })
}

fn part_path(dest: &Path) -> std::path::PathBuf {
    match dest.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => dest.with_extension(format!("{ext}.part")),
        None => dest.with_extension("part"),
    }
}

fn copy_body<W: Write>(
    resp: &mut Response,
    max_bytes: Option<u64>,
    writer: &mut W,
) -> Result<(u64, u64)> {
    let mut buf = vec![0u8; READ_BUF];
    let mut total = 0u64;
    let mut peak = 0u64;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(max) = max_bytes {
            let next = total.saturating_add(n as u64);
            if next > max {
                return Err(Error::TooLarge { limit: max, got: next });
            }
        }
        writer.write_all(&buf[..n])?;
        peak = peak.max(n as u64);
        total = total.saturating_add(n as u64);
    }
    Ok((total, peak))
}

pub fn fetch_bounded(
    cfg: &GetCfg,
    url: &str,
    range: ByteRange,
    whole_limit: u64,
) -> Result<Fetched> {
    let len = range
        .len()
        .ok_or_else(|| Error::Http("range end is before start".into()))?;
    let len_us = usize::try_from(len).map_err(|_| Error::TooLarge {
        limit: u64::MAX,
        got: len,
    })?;
    let mut resp = open(cfg, "GET", url, Some(RangeSpec::Bounded(range)))?;
    if resp.head.status == 206 {
        let mut body = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if body.len().saturating_add(n) > len_us.saturating_add(64) {
                return Err(Error::Http("206 body is longer than the range".into()));
            }
            body.extend_from_slice(&buf[..n]);
        }
        return Ok(Fetched::Part(body));
    }
    if resp.head.status != 200 {
        return Err(Error::Http(format!("status {}", resp.head.status)));
    }
    let mut acc = Vec::new();
    let mut caching = true;
    let mut part = vec![0u8; len_us];
    let mut filled = 0usize;
    let mut pos = 0u64;
    let mut buf = [0u8; 8192];
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if caching {
            let next = acc.len().saturating_add(n) as u64;
            if next > whole_limit {
                acc.clear();
                caching = false;
            } else {
                acc.extend_from_slice(&buf[..n]);
            }
        }
        let chunk_start = pos;
        let chunk_end = pos + n as u64;
        if chunk_end > range.start && chunk_start < range.end {
            let from = range.start.max(chunk_start);
            let to = range.end.min(chunk_end);
            let src_off = (from - chunk_start) as usize;
            let dst_off = (from - range.start) as usize;
            let ncopy = (to - from) as usize;
            part[dst_off..dst_off + ncopy]
                .copy_from_slice(&buf[src_off..src_off + ncopy]);
            filled = filled.max(dst_off + ncopy);
        }
        pos = chunk_end;
        if !caching && pos >= range.end {
            break;
        }
    }
    let _ = filled;
    if caching {
        Ok(Fetched::Whole(acc))
    } else {
        Ok(Fetched::Part(part))
    }
}

fn open(cfg: &GetCfg, method: &str, url: &str, range: Option<RangeSpec>) -> Result<Response> {
    let mut current = policy::parse_url(url)?;
    if current.scheme != "http" {
        return Err(Error::Unsupported(
            "cleartext http only; this crate has no TLS".into(),
        ));
    }
    let mut method = method.to_string();
    for hop in 0..=cfg.max_redirects {
        enforce(&current, cfg)?;
        let mut tcp = dial(&current, cfg)?;
        let req = build_request(&current, &method, range, false)?;
        tcp.write_all(req.as_bytes())?;
        tcp.flush()?;
        let (status, headers, pending) = read_head(&mut tcp)?;
        if is_redirect(status) {
            if hop == cfg.max_redirects {
                return Err(Error::Http("too many redirects".into()));
            }
            let loc = header(&headers, "location")
                .ok_or_else(|| Error::Http("redirect missing location".into()))?;
            current = policy::join_url(&current, loc)?;
            if current.scheme != "http" {
                return Err(Error::Unsupported(
                    "cleartext http only; this crate has no TLS".into(),
                ));
            }
            method = "GET".into();
            continue;
        }
        let head = make_head(status, &current, &headers);
        let (kind, left, chunk_need_line) = body_kind(method == "HEAD", status, &headers, &pending);
        return Ok(Response {
            head,
            tcp,
            buf: pending,
            pos: 0,
            kind,
            left,
            chunk_need_line,
            chunk_done: false,
            persist_ok: false,
        });
    }
    Err(Error::Http("too many redirects".into()))
}

fn enforce(url: &Url, cfg: &GetCfg) -> Result<()> {
    if url.scheme != "http" {
        return Err(Error::Unsupported(
            "cleartext http only; this crate has no TLS".into(),
        ));
    }
    if !cfg.check_ssrf {
        return Ok(());
    }
    if let Ok(ip) = url.host.parse::<IpAddr>() {
        allow_ip(ip, cfg)?;
    }
    Ok(())
}

fn allow_ip(ip: IpAddr, cfg: &GetCfg) -> Result<()> {
    if !cfg.check_ssrf {
        return Ok(());
    }
    if cfg.permit_loopback && ip.is_loopback() {
        return Ok(());
    }
    policy::refuse_ip(ip)
}

fn dial(url: &Url, cfg: &GetCfg) -> Result<TcpStream> {
    let addrs: Vec<_> = (url.host.as_str(), url.port)
        .to_socket_addrs_owned()
        .map_err(|err| Error::Http(format!("could not resolve host: {err}")))?;
    if addrs.is_empty() {
        return Err(Error::Http("could not resolve host".into()));
    }
    for addr in &addrs {
        allow_ip(addr.ip(), cfg)?;
    }
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, cfg.timeout) {
            Ok(stream) => {
                stream.set_read_timeout(Some(cfg.timeout))?;
                stream.set_write_timeout(Some(cfg.timeout))?;
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(err) => last = Some(err),
        }
    }
    Err(last
        .unwrap_or_else(|| std::io::Error::other("connect"))
        .into())
}

trait ToSocketAddrsOwned {
    fn to_socket_addrs_owned(&self) -> std::io::Result<Vec<std::net::SocketAddr>>;
}

impl ToSocketAddrsOwned for (&str, u16) {
    fn to_socket_addrs_owned(&self) -> std::io::Result<Vec<std::net::SocketAddr>> {
        use std::net::ToSocketAddrs;
        Ok(self.to_socket_addrs()?.collect())
    }
}

fn build_request(
    url: &Url,
    method: &str,
    range: Option<RangeSpec>,
    persist: bool,
) -> Result<String> {
    if method != "GET" && method != "HEAD" {
        return Err(Error::Http("only GET and HEAD".into()));
    }
    let connection = if persist { "keep-alive" } else { "close" };
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {ua}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: {connection}\r\n",
        path = url.path,
        host = host_header(url),
        ua = USER_AGENT,
    );
    if let Some(range) = range {
        req.push_str("Range: ");
        req.push_str(&range_value(range));
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    Ok(req)
}

fn host_header(url: &Url) -> String {
    if url.port == 80 {
        url.host_display()
    } else if url.host.contains(':') {
        format!("[{}]:{}", url.host, url.port)
    } else {
        format!("{}:{}", url.host, url.port)
    }
}

fn range_value(spec: RangeSpec) -> String {
    match spec {
        RangeSpec::From(start) => format!("bytes={start}-"),
        RangeSpec::Bounded(range) => {
            let end = range.end.saturating_sub(1);
            format!("bytes={}-{}", range.start, end)
        }
    }
}

#[allow(clippy::type_complexity)]
fn read_head(tcp: &mut TcpStream) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
    let mut raw = Vec::new();
    let mut tmp = [0u8; 1024];
    let split_at = loop {
        if raw.len() > 64 * 1024 {
            return Err(Error::Http("headers too large".into()));
        }
        let n = tcp.read(&mut tmp)?;
        if n == 0 {
            return Err(Error::Http("connection closed before headers".into()));
        }
        raw.extend_from_slice(&tmp[..n]);
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
    };
    let header_bytes = raw[..split_at].to_vec();
    let pending = raw[split_at + 4..].to_vec();
    let text = String::from_utf8_lossy(&header_bytes);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| Error::Http("bad status line".into()))?
        .parse()
        .map_err(|_| Error::Http("bad status line".into()))?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    if let Some(len_a) = header(&headers, "content-length") {
        if headers
            .iter()
            .filter(|(name, _)| name == "content-length")
            .any(|(_, value)| value != len_a)
        {
            return Err(Error::Http("conflicting content-length".into()));
        }
    }
    let chunked = header(&headers, "transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    if chunked && header(&headers, "content-length").is_some() {
        return Err(Error::Http(
            "content-length and chunked together".into(),
        ));
    }
    Ok((status, headers, pending))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn make_head(status: u16, url: &Url, headers: &[(String, String)]) -> Head {
    let content_length = header(headers, "content-length").and_then(|v| v.parse().ok());
    let content_type = header(headers, "content-type").map(|v| v.to_string());
    let accept_ranges = header(headers, "accept-ranges")
        .map(|v| v.to_ascii_lowercase().contains("bytes"))
        .unwrap_or(false);
    let content_range = header(headers, "content-range").and_then(parse_content_range);
    Head {
        status,
        final_url: url.to_string_url(),
        content_type,
        content_length,
        accept_ranges,
        content_range,
    }
}

fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let value = value.trim().strip_prefix("bytes ")?;
    let (span, total) = value.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    let end_incl: u64 = end.parse().ok()?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    Some((start, end_incl.saturating_add(1), total))
}

fn body_kind(
    head: bool,
    status: u16,
    headers: &[(String, String)],
    pending: &[u8],
) -> (Kind, u64, bool) {
    if head || status == 204 || status == 304 || is_redirect(status) {
        return (Kind::Empty, 0, false);
    }
    let chunked = header(headers, "transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    if chunked {
        return (Kind::Chunk, 0, true);
    }
    if let Some(len) = header(headers, "content-length").and_then(|v| v.parse::<u64>().ok()) {
        let pending_len = pending.len() as u64;
        let left = len;
        let _ = pending_len;
        return (Kind::Length, left, false);
    }
    (Kind::Eof, 0, false)
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn exchange(
    mut tcp: TcpStream,
    url: &Url,
    range: Option<RangeSpec>,
    persist: bool,
) -> Result<Response> {
    let req = build_request(url, "GET", range, persist)?;
    tcp.write_all(req.as_bytes())?;
    tcp.flush()?;
    let (status, headers, pending) = read_head(&mut tcp)?;
    if is_redirect(status) {
        return Err(Error::Http(
            "shared connection does not follow redirects".into(),
        ));
    }
    let head = make_head(status, url, &headers);
    let (kind, left, chunk_need_line) = body_kind(false, status, &headers, &pending);
    let persist_ok = persist && keeps_alive(&headers) && matches!(kind, Kind::Length);
    Ok(Response {
        head,
        tcp,
        buf: pending,
        pos: 0,
        kind,
        left,
        chunk_need_line,
        chunk_done: false,
        persist_ok,
    })
}

fn keeps_alive(headers: &[(String, String)]) -> bool {
    match header(headers, "connection") {
        Some(value) => !value
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("close")),
        None => true,
    }
}

impl Response {
    fn into_idle(self) -> Option<TcpStream> {
        if !self.persist_ok || self.left != 0 || self.pos != self.buf.len() {
            return None;
        }
        match self.kind {
            Kind::Length => Some(self.tcp),
            _ => None,
        }
    }

    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        match self.kind {
            Kind::Empty => Ok(0),
            Kind::Eof => self.read_raw(out),
            Kind::Length => {
                if self.left == 0 {
                    return Ok(0);
                }
                let want = (self.left as usize).min(out.len());
                let n = self.read_raw(&mut out[..want])?;
                if n == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "short body",
                    ));
                }
                self.left -= n as u64;
                Ok(n)
            }
            Kind::Chunk => self.read_chunk(out),
        }
    }

    fn read_chunk(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.chunk_done {
            return Ok(0);
        }
        if self.chunk_need_line {
            let line = self.read_line()?;
            let hex = line.split(';').next().unwrap_or("").trim();
            if hex.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "empty chunk size",
                ));
            }
            let size = u64::from_str_radix(hex, 16).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size")
            })?;
            if size == 0 {
                loop {
                    let trailer = self.read_line()?;
                    if trailer.is_empty() {
                        break;
                    }
                }
                self.chunk_done = true;
                return Ok(0);
            }
            self.left = size;
            self.chunk_need_line = false;
        }
        let want = (self.left as usize).min(out.len());
        let n = self.read_raw(&mut out[..want])?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "short chunk",
            ));
        }
        self.left -= n as u64;
        if self.left == 0 {
            let mut crlf = [0u8; 2];
            self.read_exact(&mut crlf)?;
            if &crlf != b"\r\n" {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "chunk missing crlf",
                ));
            }
            self.chunk_need_line = true;
        }
        Ok(n)
    }

    fn read_line(&mut self) -> std::io::Result<String> {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = self.read_raw(&mut byte)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "short chunk line",
                ));
            }
            if byte[0] == b'\n' {
                break;
            }
            if byte[0] != b'\r' {
                line.push(byte[0]);
            }
            if line.len() > 8192 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "chunk line too long",
                ));
            }
        }
        Ok(String::from_utf8_lossy(&line).trim().to_string())
    }

    fn read_exact(&mut self, out: &mut [u8]) -> std::io::Result<()> {
        let mut filled = 0;
        while filled < out.len() {
            let n = self.read_raw(&mut out[filled..])?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "short read",
                ));
            }
            filled += n;
        }
        Ok(())
    }

    fn read_raw(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.buf.len() {
            let n = (self.buf.len() - self.pos).min(out.len());
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        self.tcp.read(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{self, Resp};
    use std::sync::atomic::Ordering;

    fn cfg() -> GetCfg {
        GetCfg {
            timeout: Duration::from_secs(2),
            max_redirects: 4,
            max_bytes: None,
            check_ssrf: true,
            permit_loopback: true,
        }
    }

    struct Peak(u64, u64);
    impl Write for Peak {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.max(buf.len() as u64);
            self.1 += buf.len() as u64;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn streams_without_buffering_the_file() {
        let body = vec![7u8; 200_000];
        let server = harness::serve(move |req| harness::with_range(req, &body));
        let mut peak = Peak(0, 0);
        let outcome = download_to(&cfg(), &server.url("/v"), None, &mut peak).unwrap();
        assert_eq!(outcome.bytes_written, 200_000);
        assert_eq!(peak.1, 200_000);
        assert!(peak.0 <= READ_BUF as u64);
        assert!(outcome.peak_write <= READ_BUF as u64);
        assert!(outcome.peak_write < 200_000);
    }

    #[test]
    fn follows_redirects_and_refuses_private_targets() {
        let server = harness::serve(|req| {
            if req.target.starts_with("/go") {
                Resp {
                    status: 302,
                    headers: vec![("Location".into(), "/land".into())],
                    body: Vec::new(),
                    chunked: false,
                }
            } else if req.target.starts_with("/bad") {
                Resp {
                    status: 302,
                    headers: vec![("Location".into(), "http://10.1.2.3/x".into())],
                    body: Vec::new(),
                    chunked: false,
                }
            } else if req.target.starts_with("/chunk") {
                Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/plain".into())],
                    body: b"hello-chunk".to_vec(),
                    chunked: true,
                }
            } else {
                Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/plain".into())],
                    body: b"landed".to_vec(),
                    chunked: false,
                }
            }
        });
        let mut out = Vec::new();
        let outcome = download_to(&cfg(), &server.url("/go"), None, &mut out).unwrap();
        assert_eq!(out, b"landed");
        assert!(outcome.final_url.ends_with("/land"));
        let err = download_to(&cfg(), &server.url("/bad"), None, &mut Vec::new()).unwrap_err();
        assert!(matches!(err, Error::Ssrf(_)), "{err}");
        let mut chunked = Vec::new();
        download_to(&cfg(), &server.url("/chunk"), None, &mut chunked).unwrap();
        assert_eq!(chunked, b"hello-chunk");
        let raw = server.last_raw.lock().unwrap().clone();
        assert!(raw.contains(USER_AGENT));
        assert!(!raw.contains("Mozilla"));
        assert!(!raw.contains("Chrome"));
        assert!(!raw.contains("Cookie"));
    }

    #[test]
    fn resumes_and_accepts_a_full_200() {
        let body = b"0123456789abcdef".to_vec();
        let body2 = body.clone();
        let server = harness::serve(move |req| {
            if req.target.starts_with("/full") {
                Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "application/octet-stream".into())],
                    body: body2.clone(),
                    chunked: false,
                }
            } else {
                harness::with_range(req, &body2)
            }
        });
        let dir = std::env::temp_dir().join(format!(
            "export-fetch-resume-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("clip.bin");
        let part = dir.join("clip.bin.part");
        std::fs::write(&part, b"01234567").unwrap();
        let outcome = download_file(&cfg(), &server.url("/part"), &dest).unwrap();
        assert!(!outcome.restarted);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!part.exists());

        let dest2 = dir.join("full.bin");
        let part2 = dir.join("full.bin.part");
        std::fs::write(&part2, b"XXXX").unwrap();
        let outcome = download_file(&cfg(), &server.url("/full"), &dest2).unwrap();
        assert!(outcome.restarted);
        assert_eq!(std::fs::read(&dest2).unwrap(), body);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(server.hits.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn default_config_refuses_loopback_and_https() {
        let err = download_to(
            &GetCfg::default(),
            "http://127.0.0.1:9/x",
            None,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Ssrf(_)), "{err}");
        let err = download_to(
            &cfg(),
            "https://example.com/a.mp4",
            None,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{err}");
    }

    #[test]
    fn two_gets_on_one_cleartext_connection() {
        use std::io::{Read, Write};
        use std::net::{Ipv4Addr, TcpListener};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_t = Arc::clone(&accepts);
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            accepts_t.fetch_add(1, Ordering::SeqCst);
            let _ = sock.set_nodelay(true);
            for _ in 0..2 {
                let mut raw = Vec::new();
                let mut tmp = [0u8; 512];
                loop {
                    let n = sock.read(&mut tmp).unwrap();
                    raw.extend_from_slice(&tmp[..n]);
                    if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&raw);
                let body: &[u8] = if text.contains(" /b ") { b"bbb" } else { b"aaa" };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    body.len()
                );
                sock.write_all(head.as_bytes()).unwrap();
                sock.write_all(body).unwrap();
            }
        });

        let mut conn = ClearConn::new(cfg());
        let mut first = Vec::new();
        let mut second = Vec::new();
        conn.download_to(&format!("http://127.0.0.1:{}/a", addr.port()), &mut first)
            .unwrap();
        conn.download_to(&format!("http://127.0.0.1:{}/b", addr.port()), &mut second)
            .unwrap();
        assert_eq!(first, b"aaa");
        assert_eq!(second, b"bbb");
        assert_eq!(conn.connections(), 1);
        drop(conn);
        server.join().unwrap();
        assert_eq!(accepts.load(Ordering::SeqCst), 1);

        let err = ClearConn::new(GetCfg::default())
            .download_to("http://127.0.0.1:9/x", &mut Vec::new())
            .unwrap_err();
        assert!(matches!(err, Error::Ssrf(_)), "{err}");
        let err = ClearConn::new(cfg())
            .download_to("https://example.com/a.mp4", &mut Vec::new())
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{err}");
    }
}
