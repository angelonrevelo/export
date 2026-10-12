//! Cleartext loopback server for tests and the release bench.
//! Not a product entry point.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Req {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub chunked: bool,
}

pub struct Server {
    pub addr: std::net::SocketAddr,
    pub hits: Arc<AtomicUsize>,
    pub last_raw: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub fn serve<F>(handler: F) -> Server
where
    F: Fn(&Req) -> Resp + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let last_raw = Arc::new(Mutex::new(String::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let hits_t = Arc::clone(&hits);
    let last_t = Arc::clone(&last_raw);
    let stop_t = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        let handler = Arc::new(handler);
        loop {
            match listener.accept() {
                Ok((sock, _)) => {
                    if stop_t.load(Ordering::SeqCst) {
                        break;
                    }
                    hits_t.fetch_add(1, Ordering::SeqCst);
                    let handler = Arc::clone(&handler);
                    let last = Arc::clone(&last_t);
                    thread::spawn(move || {
                        let _ = handle_one(sock, handler.as_ref(), &last);
                    });
                }
                Err(_) => break,
            }
        }
    });
    Server {
        addr,
        hits,
        last_raw,
        stop,
        thread: Some(thread),
    }
}

impl Server {
    pub fn url(&self, path: &str) -> String {
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("http://127.0.0.1:{}{path}", self.addr.port())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(400));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_one<F: Fn(&Req) -> Resp>(
    mut sock: TcpStream,
    handler: &F,
    last: &Mutex<String>,
) -> std::io::Result<()> {
    let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = sock.set_write_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 64 * 1024 {
            break;
        }
        let n = match sock.read(&mut tmp) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(_) => return Ok(()),
        };
        buf.extend_from_slice(&tmp[..n]);
    }
    let raw = String::from_utf8_lossy(&buf).into_owned();
    if let Ok(mut guard) = last.lock() {
        *guard = raw.clone();
    }
    let req = parse_req(&raw);
    let resp = handler(&req);
    write_resp(&mut sock, &resp)
}

fn parse_req(raw: &str) -> Req {
    let mut lines = raw.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    Req {
        method,
        target,
        headers,
    }
}

fn write_resp(sock: &mut TcpStream, resp: &Resp) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} OK\r\nConnection: close\r\n", resp.status);
    let mut has_len = false;
    let mut has_te = false;
    for (name, value) in &resp.headers {
        if name.eq_ignore_ascii_case("content-length") {
            has_len = true;
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            has_te = true;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    if resp.chunked {
        if !has_te {
            head.push_str("Transfer-Encoding: chunked\r\n");
        }
    } else if !has_len && resp.method_sends_body() {
        head.push_str(&format!("Content-Length: {}\r\n", resp.body.len()));
    }
    head.push_str("\r\n");
    sock.write_all(head.as_bytes())?;
    if resp.chunked {
        for chunk in resp.body.chunks(5) {
            write!(sock, "{:x}\r\n", chunk.len())?;
            sock.write_all(chunk)?;
            sock.write_all(b"\r\n")?;
        }
        sock.write_all(b"0\r\n\r\n")?;
    } else if resp.method_sends_body() {
        sock.write_all(&resp.body)?;
    }
    Ok(())
}

impl Resp {
    fn method_sends_body(&self) -> bool {
        self.status != 204 && self.status != 304
    }
}

pub fn with_range(req: &Req, body: &[u8]) -> Resp {
    if req.method.eq_ignore_ascii_case("HEAD") {
        return Resp {
            status: 200,
            headers: vec![
                ("Accept-Ranges".into(), "bytes".into()),
                ("Content-Type".into(), "application/octet-stream".into()),
                ("Content-Length".into(), body.len().to_string()),
            ],
            body: Vec::new(),
            chunked: false,
        };
    }
    if let Some(spec) = req
        .headers
        .iter()
        .find(|(name, _)| name == "range")
        .and_then(|(_, value)| parse_range_header(value))
    {
        if spec.0 >= body.len() {
            return Resp {
                status: 416,
                headers: vec![],
                body: Vec::new(),
                chunked: false,
            };
        }
        let end = spec.1.unwrap_or(body.len()).min(body.len()).max(spec.0);
        let slice = body[spec.0..end].to_vec();
        let last = end.saturating_sub(1);
        return Resp {
            status: 206,
            headers: vec![
                ("Accept-Ranges".into(), "bytes".into()),
                (
                    "Content-Range".into(),
                    format!("bytes {}-{}/{}", spec.0, last, body.len()),
                ),
                ("Content-Type".into(), "application/octet-stream".into()),
            ],
            body: slice,
            chunked: false,
        };
    }
    Resp {
        status: 200,
        headers: vec![
            ("Accept-Ranges".into(), "bytes".into()),
            ("Content-Type".into(), "application/octet-stream".into()),
        ],
        body: body.to_vec(),
        chunked: false,
    }
}

fn parse_range_header(value: &str) -> Option<(usize, Option<usize>)> {
    let value = value.trim().strip_prefix("bytes=")?;
    let (start, end) = value.split_once('-')?;
    let start = start.parse().ok()?;
    if end.is_empty() {
        return Some((start, None));
    }
    let end_incl: usize = end.parse().ok()?;
    Some((start, Some(end_incl.saturating_add(1))))
}
