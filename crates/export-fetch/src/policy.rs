//! URL parsing and the Export SSRF refusal.
//!
//! Private, loopback, link-local, multicast, and the same reserved ranges
//! `app/ssrf.py` blocks are refused. This crate does not store credentials.

use crate::error::{Error, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// Path plus query, always starting with `/`. No fragment.
    pub path: String,
}

impl Url {
    pub fn origin(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        if self.port == default {
            format!("{}://{}", self.scheme, self.host_display())
        } else {
            format!("{}://{}:{}", self.scheme, self.host_display(), self.port)
        }
    }

    pub fn to_string_url(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }

    pub fn host_display(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }

    pub fn default_port(scheme: &str) -> Option<u16> {
        match scheme {
            "http" => Some(80),
            "https" => Some(443),
            "socks5" => Some(1080),
            _ => None,
        }
    }
}

pub fn parse_url(input: &str) -> Result<Url> {
    let input = input.trim();
    let (scheme, rest) = input
        .split_once("://")
        .ok_or_else(|| Error::Url("only http/https URLs".into()))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" && scheme != "socks5" {
        return Err(Error::Url("only http/https URLs".into()));
    }
    if rest.is_empty() {
        return Err(Error::Url("URL has no host".into()));
    }
    let (authority, path_q) = match rest.find(['/', '?', '#']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(Error::Url("URL has no host".into()));
    }
    if authority.contains('@') {
        return Err(Error::Url(
            "credentials belong in the cookie or proxy config, not the URL".into(),
        ));
    }
    let (host, port) = split_host_port(authority)?;
    if host.is_empty() {
        return Err(Error::Url("URL has no host".into()));
    }
    let port = if let Some(p) = port {
        p.parse::<u16>()
            .map_err(|_| Error::Url("bad port".into()))?
    } else {
        Url::default_port(&scheme).ok_or_else(|| Error::Url("bad scheme".into()))?
    };
    let mut path = if path_q.is_empty() {
        "/".to_string()
    } else if path_q.starts_with('?') {
        format!("/{path_q}")
    } else {
        path_q.to_string()
    };
    if let Some(i) = path.find('#') {
        path.truncate(i);
    }
    if path.is_empty() {
        path = "/".to_string();
    }
    Ok(Url {
        scheme,
        host,
        port,
        path,
    })
}

fn split_host_port(authority: &str) -> Result<(String, Option<&str>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| Error::Url("bad ipv6 host".into()))?;
        let host = &rest[..end];
        let after = &rest[end + 1..];
        if after.is_empty() {
            return Ok((host.to_string(), None));
        }
        let port = after
            .strip_prefix(':')
            .ok_or_else(|| Error::Url("bad ipv6 host".into()))?;
        return Ok((host.to_string(), Some(port)));
    }
    if authority.matches(':').count() > 1 {
        return Err(Error::Url("bad host".into()));
    }
    if let Some((host, port)) = authority.rsplit_once(':') {
        Ok((host.to_string(), Some(port)))
    } else {
        Ok((authority.to_string(), None))
    }
}

/// Join a redirect or an HTML reference against a base URL.
pub fn join_url(base: &Url, loc: &str) -> Result<Url> {
    let loc = loc.trim();
    if loc.is_empty() {
        return Err(Error::Http("empty redirect".into()));
    }
    if loc.contains('\r') || loc.contains('\n') {
        return Err(Error::Http("redirect location rejected".into()));
    }
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return parse_url(loc);
    }
    if loc.starts_with("//") {
        return parse_url(&format!("{}:{loc}", base.scheme));
    }
    let (path_part, query) = match loc.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (loc, None),
    };
    let path_part = path_part.split('#').next().unwrap_or(path_part);
    let mut combined = if path_part.starts_with('/') {
        path_part.to_string()
    } else {
        let dir = base.path.split('?').next().unwrap_or("/");
        let dir = match dir.rsplit_once('/') {
            Some((d, _)) => d,
            None => "",
        };
        if path_part.is_empty() {
            dir.to_string()
        } else {
            format!("{dir}/{path_part}")
        }
    };
    combined = normalize_path(&combined);
    if let Some(q) = query {
        let q = q.split('#').next().unwrap_or(q);
        combined.push('?');
        combined.push_str(q);
    } else if path_part.is_empty() {
        if let Some(q) = base.path.split_once('?').map(|(_, q)| q) {
            combined.push('?');
            combined.push_str(q);
        }
    }
    Ok(Url {
        scheme: base.scheme.clone(),
        host: base.host.clone(),
        port: base.port,
        path: combined,
    })
}

fn normalize_path(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            out.pop();
            continue;
        }
        out.push(seg);
    }
    let mut s = String::from("/");
    s.push_str(&out.join("/"));
    s
}

pub fn refuse_ip(ip: IpAddr) -> Result<()> {
    if is_blocked_ip(ip) {
        Err(Error::Ssrf(
            "refused: private or metadata address".into(),
        ))
    } else {
        Ok(())
    }
}

pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
    {
        return true;
    }
    let o = ip.octets();
    // Documentation ranges (TEST-NET), 0.0.0.0/8, 100.64.0.0/10, 192.0.0.0/29,
    // 198.18.0.0/15, 240.0.0.0/4. Same refusal set as app/ssrf.py.
    if o[0] == 0 || o[0] >= 240 {
        return true;
    }
    if o[0] == 100 && (o[1] & 0xc0) == 64 {
        return true;
    }
    if o[0] == 192 && o[1] == 0 && o[2] == 0 && o[3] <= 7 {
        return true;
    }
    if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
        return true;
    }
    // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
    if o[0] == 192 && o[1] == 0 && o[2] == 2 {
        return true;
    }
    if o[0] == 198 && o[1] == 51 && o[2] == 100 {
        return true;
    }
    if o[0] == 203 && o[1] == 0 && o[2] == 113 {
        return true;
    }
    false
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_multicast() || ip.is_unspecified() {
        return true;
    }
    let s = ip.segments();
    // fc00::/7 unique local, fe80::/10 link-local.
    if (s[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    if (s[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // IPv4-mapped: ::ffff:0:0/96. A public mapped address stays public.
    if s[0..5] == [0, 0, 0, 0, 0] && s[5] == 0xffff {
        let o = ip.octets();
        return is_blocked_v4(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
    }
    // 2001:db8::/32 documentation.
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return true;
    }
    false
}

/// Refuse a URL whose literal address is private. DNS resolution happens at
/// connect time so every answer can be checked, including after a redirect.
pub fn refuse_ssrf(url: &str) -> Result<()> {
    let parsed = parse_url(url)?;
    if parsed.scheme != "http" && parsed.scheme != "https" {
        return Err(Error::Ssrf("only http/https URLs".into()));
    }
    if let Ok(ip) = parsed.host.parse::<IpAddr>() {
        refuse_ip(ip)?;
    }
    Ok(())
}

pub fn extension_of(path: &str) -> Option<String> {
    let path = path.split('?').next().unwrap_or(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 8 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

pub fn sanitize_stem(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c == ' ' || c == '.' {
            out.push('_');
        }
        if out.len() >= 80 {
            break;
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "download".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_joins() {
        let u = parse_url("https://Example.COM:8443/a/b?x=1#frag").unwrap();
        assert_eq!(u.host, "Example.COM");
        assert_eq!(u.port, 8443);
        assert_eq!(u.path, "/a/b?x=1");
        let next = join_url(&u, "../c").unwrap();
        assert_eq!(next.path, "/c");
        let abs = join_url(&u, "/z").unwrap();
        assert_eq!(abs.path, "/z");
        let host = join_url(&u, "//cdn.example/v.mp4").unwrap();
        assert_eq!(host.host, "cdn.example");
        assert_eq!(host.scheme, "https");
    }

    #[test]
    fn refuses_userinfo_and_bad_scheme() {
        assert!(parse_url("http://user:pw@example.com/").is_err());
        assert!(parse_url("file:///etc/passwd").is_err());
        assert!(parse_url("http:///nohost").is_err());
    }

    #[test]
    fn ssrf_blocks_private_loopback_link_local() {
        for url in [
            "http://127.0.0.1/x",
            "http://127.0.0.2/",
            "http://10.1.2.3/a",
            "http://192.168.0.1/",
            "http://172.16.0.1/",
            "http://169.254.169.254/latest",
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://255.255.255.255/",
            "http://224.0.0.1/",
            "http://100.64.0.1/",
            "http://198.18.0.1/",
            "http://240.0.0.1/",
        ] {
            assert!(refuse_ssrf(url).is_err(), "{url}");
        }
        assert!(refuse_ssrf("https://example.com/a.mp4").is_ok());
    }

    #[test]
    fn ipv6_literal_parses() {
        let u = parse_url("http://[::1]:9/a").unwrap();
        assert_eq!(u.host, "::1");
        assert_eq!(u.port, 9);
        assert!(refuse_ssrf("http://[::1]:9/a").is_err());
    }
}
