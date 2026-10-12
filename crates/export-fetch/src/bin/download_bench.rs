//! Release-only cleartext GET of 8 MiB from a `std::net` server on 127.0.0.1.
//!
//! Loopback is permitted only in this bench's `GetCfg`. The library default
//! still refuses it. `https` is not exercised and stays unsupported.
//! Debug builds print NOT JUDGED and exit 2.

use export_fetch::{download_to, GetCfg};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const PAYLOAD: usize = 8 * 1024 * 1024;
const SAMPLES: usize = 40;

fn main() {
    if cfg!(debug_assertions) {
        println!("NOT JUDGED");
        std::process::exit(2);
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind 127.0.0.1");
    let addr = listener.local_addr().expect("local addr");
    assert!(
        addr.ip().is_loopback(),
        "bench server must stay on loopback"
    );
    let stop = Arc::new(AtomicBool::new(false));
    let stop_srv = Arc::clone(&stop);
    let server = thread::spawn(move || serve(listener, stop_srv, Arc::new(vec![0x5au8; PAYLOAD])));

    let url = format!("http://127.0.0.1:{}/payload.bin", addr.port());
    let cfg = bench_cfg();
    let mut sink = Sink { acc: 0, bytes: 0 };
    let warm = download_to(&cfg, &url, None, &mut sink).expect("warmup GET");
    assert_eq!(warm.status, 200);
    assert_eq!(warm.bytes_written, PAYLOAD as u64);
    assert!(warm.peak_write <= 65_536);

    let mut wall_ns = Vec::with_capacity(SAMPLES);
    let mut cpu_ns = Vec::with_capacity(SAMPLES);
    let mut peak = 0u64;
    for _ in 0..SAMPLES {
        sink.bytes = 0;
        let cpu0 = cpu_ns_now();
        let wall0 = Instant::now();
        let outcome = download_to(&cfg, &url, None, &mut sink).expect("GET");
        let elapsed = wall0.elapsed().as_nanos() as u64;
        let cpu = cpu_ns_now().saturating_sub(cpu0);
        assert_eq!(outcome.status, 200);
        assert_eq!(outcome.bytes_written, PAYLOAD as u64);
        assert!(outcome.final_url.starts_with("http://127.0.0.1:"));
        wall_ns.push(elapsed);
        cpu_ns.push(cpu);
        peak = peak.max(outcome.peak_write);
        std::hint::black_box(sink.acc);
    }

    stop.store(true, Ordering::SeqCst);
    if let Ok(mut poke) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        let _ = poke.write_all(b"\r\n\r\n");
        let _ = poke.shutdown(Shutdown::Both);
    }
    let _ = server.join();

    wall_ns.sort_unstable();
    cpu_ns.sort_unstable();
    let p50 = percentile(&wall_ns, 50);
    let p99 = percentile(&wall_ns, 99);
    let cpu_p50 = percentile(&cpu_ns, 50);
    let cpu_p99 = percentile(&cpu_ns, 99);
    let mb = (PAYLOAD as f64) / (1024.0 * 1024.0);
    let bps = |ns: u64| -> f64 {
        if ns == 0 {
            return 0.0;
        }
        (PAYLOAD as f64) / (ns as f64 / 1e9)
    };
    let per_mb = |ns: u64| -> f64 { (ns as f64) / mb };

    println!("bench=cleartext_get");
    println!("release=1");
    println!("scheme=http");
    println!("host=127.0.0.1");
    println!("permit_loopback=bench_only");
    println!("https=unsupported");
    println!("payload_bytes={PAYLOAD}");
    println!("samples={SAMPLES}");
    println!("warmup=1");
    println!("bytes_per_sec_p50={:.0}", bps(p50));
    println!("bytes_per_sec_p99={:.0}", bps(p99));
    println!("latency_ns_p50={p50}");
    println!("latency_ns_p99={p99}");
    println!("process_cpu_ns_per_mb_p50={:.0}", per_mb(cpu_p50));
    println!("process_cpu_ns_per_mb_p99={:.0}", per_mb(cpu_p99));
    println!("peak_buffered_bytes={peak}");
    println!("vm_hwm_kib={}", vm_hwm_kib().unwrap_or(0));
    println!("bytes_checked={}", sink.bytes);
    println!("sink_check={}", sink.acc);
    let _ = io::stdout().flush();
}

fn bench_cfg() -> GetCfg {
    GetCfg {
        timeout: Duration::from_secs(30),
        max_redirects: 0,
        max_bytes: Some(PAYLOAD as u64),
        check_ssrf: true,
        permit_loopback: true,
    }
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>, body: Arc<Vec<u8>>) {
    while !stop.load(Ordering::SeqCst) {
        let mut sock = match listener.accept() {
            Ok((sock, peer)) => {
                if peer.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
                    continue;
                }
                sock
            }
            Err(_) => break,
        };
        let _ = sock.set_nodelay(true);
        let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = sock.set_write_timeout(Some(Duration::from_secs(30)));
        let _ = respond(&mut sock, &body);
    }
}

fn respond(sock: &mut TcpStream, body: &[u8]) -> io::Result<()> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = sock.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if raw.len() > 64 * 1024 {
            return Ok(());
        }
    }
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
        len = body.len()
    );
    sock.write_all(header.as_bytes())?;
    sock.write_all(body)?;
    sock.flush()?;
    Ok(())
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

fn cpu_ns_now() -> u64 {
    let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
    let rc = unsafe { clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    if rc != 0 {
        return 0;
    }
    (ts.tv_sec as u64).saturating_mul(1_000_000_000) + ts.tv_nsec as u64
}

fn vm_hwm_kib() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;

extern "C" {
    fn clock_gettime(clk_id: i32, tp: *mut Timespec) -> i32;
}
