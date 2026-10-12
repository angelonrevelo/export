//! Two cleartext 1 MiB GETs on the local stack, not a CDN.
//!
//! Times both bodies on one connection, then the same two with one connection
//! per GET. Loopback is permitted only in this bench. `https` stays unsupported.
//! Debug builds print NOT JUDGED and exit 2.
//!
//! The shared connection stays in the library only when its latency p50 is at
//! least 10% under the separate-connection p50. This bin records both.

use export_fetch::{download_to, ClearConn, GetCfg};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const EACH: usize = 1024 * 1024;
const COMBINED: u64 = (EACH * 2) as u64;
const SAMPLES: usize = 40;

fn main() {
    if cfg!(debug_assertions) {
        println!("NOT JUDGED");
        std::process::exit(2);
    }

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

    let a = format!("http://127.0.0.1:{}/a.bin", addr.port());
    let b = format!("http://127.0.0.1:{}/b.bin", addr.port());
    let cfg = bench_cfg();

    let _ = time_shared(&cfg, &a, &b);
    let _ = time_separate(&cfg, &a, &b);

    let mut shared_ns = Vec::with_capacity(SAMPLES);
    let mut separate_ns = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        if i % 2 == 0 {
            shared_ns.push(time_shared(&cfg, &a, &b));
            separate_ns.push(time_separate(&cfg, &a, &b));
        } else {
            separate_ns.push(time_separate(&cfg, &a, &b));
            shared_ns.push(time_shared(&cfg, &a, &b));
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

    println!("bench=cleartext_pair");
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
    print_mode("shared", 1, shared_p50, shared_p99);
    print_mode("separate", 2, separate_p50, separate_p99);
    println!(
        "rule=keep_shared_when_p50_at_least_10_percent_under_separate"
    );
    println!("shared_p50_ns={shared_p50}");
    println!("separate_p50_ns={separate_p50}");
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

fn time_shared(cfg: &GetCfg, a: &str, b: &str) -> u64 {
    let mut conn = ClearConn::new(cfg.clone());
    let mut sink = Sink { acc: 0, bytes: 0 };
    let wall0 = Instant::now();
    let first = conn.download_to(a, &mut sink).expect("shared a");
    let second = conn.download_to(b, &mut sink).expect("shared b");
    let elapsed = wall0.elapsed().as_nanos() as u64;
    assert_eq!(conn.connections(), 1);
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
    assert_eq!(first.bytes_written, EACH as u64);
    assert_eq!(second.bytes_written, EACH as u64);
    assert_eq!(sink.bytes, COMBINED);
    assert!(first.peak_write <= 65_536 && second.peak_write <= 65_536);
    std::hint::black_box(sink.acc);
    elapsed
}

fn time_separate(cfg: &GetCfg, a: &str, b: &str) -> u64 {
    let mut sink = Sink { acc: 0, bytes: 0 };
    let wall0 = Instant::now();
    let first = download_to(cfg, a, None, &mut sink).expect("separate a");
    let second = download_to(cfg, b, None, &mut sink).expect("separate b");
    let elapsed = wall0.elapsed().as_nanos() as u64;
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
    assert_eq!(first.bytes_written, EACH as u64);
    assert_eq!(second.bytes_written, EACH as u64);
    assert_eq!(sink.bytes, COMBINED);
    assert!(first.peak_write <= 65_536 && second.peak_write <= 65_536);
    std::hint::black_box(sink.acc);
    elapsed
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, accepts: Arc<AtomicUsize>) {
    let a = vec![0x11u8; EACH];
    let b = vec![0x22u8; EACH];
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
            let body = if req.starts_with("GET /b.bin ") { &b } else { &a };
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
