//! Sixteen cleartext 1 MiB GETs on the local stack, not a CDN.
//!
//! Times all sixteen bodies on one connection, then the same sixteen with one
//! connection per GET. Loopback is permitted only in this bench. `https` stays
//! unsupported. Debug builds print NOT JUDGED and exit 2.
//!
//! The shared connection stays for this size only when its latency p50 is at
//! least 10% under the sixteen-connection p50.

use export_fetch::{download_to, ClearConn, GetCfg};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const EACH: usize = 1024 * 1024;
const COUNT: usize = 16;
const COMBINED: u64 = (EACH * COUNT) as u64;
const SAMPLES: usize = 40;
const NAMES: [&str; COUNT] = [
    "a.bin", "b.bin", "c.bin", "d.bin", "e.bin", "f.bin", "g.bin", "h.bin", "i.bin", "j.bin",
    "k.bin", "l.bin", "m.bin", "n.bin", "o.bin", "p.bin",
];

fn main() {
    if cfg!(debug_assertions) {
        println!("NOT JUDGED");
        std::process::exit(2);
    }
    assert_eq!(COMBINED, 16_777_216);

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind 127.0.0.1");
    let addr = listener.local_addr().expect("local addr");
    assert!(addr.ip().is_loopback());
    let accepts = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let server = {
        let accepts = Arc::clone(&accepts);
        let stop = Arc::clone(&stop);
        thread::spawn(move || serve(listener, stop, accepts))
    };

    let urls: Vec<String> = NAMES
        .into_iter()
        .map(|name| format!("http://127.0.0.1:{}/{name}", addr.port()))
        .collect();
    let cfg = bench_cfg();

    let _ = time_shared(&cfg, &urls);
    let _ = time_separate(&cfg, &urls);

    let mut shared_ns = Vec::with_capacity(SAMPLES);
    let mut separate_ns = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        if i % 2 == 0 {
            shared_ns.push(time_shared(&cfg, &urls));
            separate_ns.push(time_separate(&cfg, &urls));
        } else {
            separate_ns.push(time_separate(&cfg, &urls));
            shared_ns.push(time_shared(&cfg, &urls));
        }
    }

    stop.store(true, Ordering::SeqCst);
    if let Ok(mut poke) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        let _ = poke.write_all(b"\r\n\r\n");
        let _ = poke.shutdown(Shutdown::Both);
    }
    let _ = server.join();

    shared_ns.sort_unstable();
    separate_ns.sort_unstable();
    let shared_p50 = percentile(&shared_ns, 50);
    let shared_p99 = percentile(&shared_ns, 99);
    let separate_p50 = percentile(&separate_ns, 50);
    let separate_p99 = percentile(&separate_ns, 99);
    let keep_shared =
        separate_p50 > 0 && shared_p50.saturating_mul(10) <= separate_p50.saturating_mul(9);

    println!("bench=cleartext_sixteen");
    println!("release=1");
    println!("scheme=http");
    println!("host=127.0.0.1");
    println!("permit_loopback=bench_only");
    println!("https=unsupported");
    println!("stack=local");
    println!("cdn=0");
    println!("samples={SAMPLES}");
    println!("warmup=1");
    println!("each_bytes={EACH}");
    println!("gets={COUNT}");
    print_mode("shared", 1, shared_p50, shared_p99);
    print_mode("separate", COUNT as u64, separate_p50, separate_p99);
    println!("rule=keep_shared_when_p50_at_least_10_percent_under_separate");
    println!("shared_p50_ns={shared_p50}");
    println!("separate_p50_ns={separate_p50}");
    println!("shared_p99_ns={shared_p99}");
    println!("separate_p99_ns={separate_p99}");
    println!("kept={}", if keep_shared { "shared" } else { "separate" });
    println!("server_accepts={}", accepts.load(Ordering::SeqCst));
    let _ = io::stdout().flush();
}

fn print_mode(mode: &str, connections: u64, p50: u64, p99: u64) {
    println!("mode={mode}");
    println!("combined_bytes={COMBINED}");
    println!("connections={connections}");
    println!("latency_ns_p50={p50}");
    println!("latency_ns_p99={p99}");
}

fn bench_cfg() -> GetCfg {
    GetCfg {
        timeout: Duration::from_secs(30),
        max_redirects: 0,
        max_bytes: Some(EACH as u64),
        check_ssrf: true,
        permit_loopback: true,
    }
}

fn time_shared(cfg: &GetCfg, urls: &[String]) -> u64 {
    let mut conn = ClearConn::new(cfg.clone());
    let mut sink = Sink { acc: 0, bytes: 0 };
    let wall0 = Instant::now();
    let mut peaks = Vec::with_capacity(urls.len());
    for url in urls {
        let outcome = conn.download_to(url, &mut sink).expect("shared get");
        assert_eq!(outcome.status, 200);
        assert_eq!(outcome.bytes_written, EACH as u64);
        peaks.push(outcome.peak_write);
    }
    let elapsed = wall0.elapsed().as_nanos() as u64;
    assert_eq!(conn.connections(), 1);
    assert_eq!(sink.bytes, COMBINED);
    assert!(peaks.iter().all(|peak| *peak <= 65_536));
    std::hint::black_box(sink.acc);
    elapsed
}

fn time_separate(cfg: &GetCfg, urls: &[String]) -> u64 {
    let mut sink = Sink { acc: 0, bytes: 0 };
    let wall0 = Instant::now();
    let mut peaks = Vec::with_capacity(urls.len());
    for url in urls {
        let outcome = download_to(cfg, url, None, &mut sink).expect("separate get");
        assert_eq!(outcome.status, 200);
        assert_eq!(outcome.bytes_written, EACH as u64);
        peaks.push(outcome.peak_write);
    }
    let elapsed = wall0.elapsed().as_nanos() as u64;
    assert_eq!(sink.bytes, COMBINED);
    assert!(peaks.iter().all(|peak| *peak <= 65_536));
    std::hint::black_box(sink.acc);
    elapsed
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, accepts: Arc<AtomicUsize>) {
    let bodies: Vec<Vec<u8>> = (0..COUNT)
        .map(|i| vec![0x11u8.wrapping_add(i as u8); EACH])
        .collect();
    while !stop.load(Ordering::SeqCst) {
        let mut sock = match listener.accept() {
            Ok((sock, peer)) => {
                if peer.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
                    continue;
                }
                accepts.fetch_add(1, Ordering::SeqCst);
                sock
            }
            Err(_) => break,
        };
        let _ = sock.set_nodelay(true);
        let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = sock.set_write_timeout(Some(Duration::from_secs(30)));
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let req = match read_request(&mut sock) {
                Ok(req) => req,
                Err(_) => break,
            };
            let close = req
                .split("\r\n")
                .any(|line| line.eq_ignore_ascii_case("Connection: close"));
            let body = &bodies[body_index(&req)];
            let connection = if close { "close" } else { "keep-alive" };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nContent-Type: application/octet-stream\r\nConnection: {connection}\r\n\r\n",
                len = body.len()
            );
            if sock.write_all(header.as_bytes()).is_err() || sock.write_all(body).is_err() {
                break;
            }
            if close {
                break;
            }
        }
    }
}

fn body_index(req: &str) -> usize {
    let path = req.split_whitespace().nth(1).unwrap_or("/a.bin");
    let letter = path.as_bytes().get(1).copied().unwrap_or(b'a');
    if (b'a'..=b'p').contains(&letter) {
        (letter - b'a') as usize
    } else {
        0
    }
}

fn read_request(sock: &mut TcpStream) -> io::Result<String> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = sock.read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof"));
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if raw.len() > 64 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "headers"));
        }
    }
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

struct Sink {
    acc: u64,
    bytes: u64,
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !buf.is_empty() {
            self.acc = std::hint::black_box(self.acc.wrapping_add(buf[0] as u64));
        }
        self.bytes += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) * pct) / 100;
    sorted[idx]
}
