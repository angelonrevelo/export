use crate::error::Error;

/// Absolute cleartext `http` URL with no userinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrl {
    pub host: String,
    pub port: u16,
    /// Path and query, always starting with `/`. The fragment is dropped.
    pub path_and_query: String,
}

impl HttpUrl {
    pub fn origin_form(&self) -> &str {
        &self.path_and_query
    }

    pub fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Accept a playlist media URI.
///
/// Relative URIs are allowed. Absolute URIs must be `http`. `https`, any
/// other scheme, userinfo, cookie query keys, and credential query keys
/// return [`Error::Unsupported`].
pub fn check_media_uri(uri: &str) -> Result<(), Error> {
    if uri.is_empty()
        || uri
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return Err(Error::BadTag);
    }
    match split_scheme(uri) {
        Some((scheme, rest)) => {
            if scheme.eq_ignore_ascii_case("https") {
                return Err(Error::Unsupported);
            }
            if !scheme.eq_ignore_ascii_case("http") {
                return Err(Error::Unsupported);
            }
            let after = rest.strip_prefix("//").ok_or(Error::BadTag)?;
            check_authority(after)?;
        }
        None => {
            if uri.starts_with("//") {
                return Err(Error::Unsupported);
            }
        }
    }
    if query_has_secret(uri) {
        return Err(Error::Unsupported);
    }
    Ok(())
}

/// Parse an absolute cleartext URL used to fetch a playlist or segment.
pub fn parse_http_url(url: &str) -> Result<HttpUrl, Error> {
    check_media_uri(url)?;
    let (scheme, rest) = split_scheme(url).ok_or(Error::BadTag)?;
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(Error::Unsupported);
    }
    let after = rest.strip_prefix("//").ok_or(Error::BadTag)?;
    let (authority, path) = split_authority(after)?;
    let (host, port) = split_host_port(authority)?;
    let path_and_query = if path.is_empty() {
        "/".to_string()
    } else {
        strip_fragment(path).to_string()
    };
    Ok(HttpUrl {
        host,
        port,
        path_and_query,
    })
}

fn split_scheme(uri: &str) -> Option<(&str, &str)> {
    let bytes = uri.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    if uri[i..].starts_with("://") {
        // Remainder starts at the "//" so callers can require a hierarchical URL.
        Some((&uri[..i], &uri[i + 1..]))
    } else {
        None
    }
}

fn check_authority(after_slashes: &str) -> Result<(), Error> {
    let (authority, _) = split_authority(after_slashes)?;
    let (_host, _port) = split_host_port(authority)?;
    Ok(())
}

fn split_authority(after_slashes: &str) -> Result<(&str, &str), Error> {
    if after_slashes.is_empty() {
        return Err(Error::BadTag);
    }
    let end = after_slashes
        .find(['/', '?', '#'])
        .unwrap_or(after_slashes.len());
    let authority = &after_slashes[..end];
    if authority.is_empty() || authority.contains('@') {
        if authority.contains('@') {
            return Err(Error::Unsupported);
        }
        return Err(Error::BadTag);
    }
    Ok((authority, &after_slashes[end..]))
}

fn split_host_port(authority: &str) -> Result<(String, u16), Error> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (addr, after) = rest.split_once(']').ok_or(Error::BadTag)?;
        if addr.is_empty() || addr.contains('[') {
            return Err(Error::BadTag);
        }
        let port = if after.is_empty() {
            80
        } else {
            let digits = after.strip_prefix(':').ok_or(Error::BadTag)?;
            parse_port(digits)?
        };
        return Ok((format!("[{addr}]"), port));
    }
    if authority.contains(':') {
        let (host, port) = authority.rsplit_once(':').ok_or(Error::BadTag)?;
        if host.is_empty() || host.contains(':') {
            return Err(Error::BadTag);
        }
        return Ok((host.to_string(), parse_port(port)?));
    }
    Ok((authority.to_string(), 80))
}

fn parse_port(port: &str) -> Result<u16, Error> {
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::BadTag);
    }
    let value: u32 = port.parse().map_err(|_| Error::BadTag)?;
    if value == 0 || value > 65535 {
        return Err(Error::BadTag);
    }
    Ok(value as u16)
}

fn strip_fragment(path: &str) -> &str {
    path.split('#').next().unwrap_or(path)
}

fn query_has_secret(uri: &str) -> bool {
    let Some((_, after)) = uri.split_once('?') else {
        return false;
    };
    let query = after.split('#').next().unwrap_or(after);
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let key = pair.split('=').next().unwrap_or(pair);
        if is_secret_name(key) {
            return true;
        }
        let decoded = percent_decode(key);
        if decoded != key && is_secret_name(&decoded) {
            return true;
        }
    }
    false
}

fn is_secret_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("cookies")
        || name.eq_ignore_ascii_case("set-cookie")
        || name.eq_ignore_ascii_case("password")
        || name.eq_ignore_ascii_case("passwd")
        || name.eq_ignore_ascii_case("pwd")
        || name.eq_ignore_ascii_case("credential")
        || name.eq_ignore_ascii_case("credentials")
        || name.eq_ignore_ascii_case("authorization")
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_userinfo_and_cookies_are_unsupported() {
        assert_eq!(
            check_media_uri("https://cdn.example/a.ts"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            check_media_uri("HTTPS://cdn.example/a.ts"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            check_media_uri("http://user:pass@cdn.example/a.ts"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            check_media_uri("http://user@cdn.example/a.ts"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            check_media_uri("http://cdn.example/a.ts?cookie=1"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            check_media_uri("http://cdn.example/a.ts?password=secret"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            check_media_uri("http://cdn.example/a.ts?%63ookie=1"),
            Err(Error::Unsupported)
        );
    }

    #[test]
    fn cleartext_and_relative_uris_pass() {
        assert_eq!(check_media_uri("seg.ts"), Ok(()));
        assert_eq!(check_media_uri("/media/seg.ts"), Ok(()));
        let url = parse_http_url("http://cdn.example/a.ts?x=1").unwrap();
        assert_eq!(url.host, "cdn.example");
        assert_eq!(url.port, 80);
        assert_eq!(url.path_and_query, "/a.ts?x=1");
        let v6 = parse_http_url("http://[2001:db8::1]:8080/a.ts").unwrap();
        assert_eq!(v6.host, "[2001:db8::1]");
        assert_eq!(v6.port, 8080);
    }
}
