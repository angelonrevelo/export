//! Header-scrub throughput.
//!
//! ```text
//! cargo bench --manifest-path crates/export-meta/Cargo.toml --bench scrub
//! ```
//!
//! Release only. A debug build prints `NOT JUDGED` and does not publish a rate.
//! The timed call is [`export_meta::scrub`]: checksum the header, rewrite it,
//! do not read the payload. The MB/s figure is file bytes covered per second
//! of that header edit, not bytes read. A `verify` row reads every payload
//! byte and is not the scrub result.

use std::hint::black_box;
use std::io::Cursor;

use export_meta::{scrub, verify, write, Record};

fn main() {
    let mut buf = sample_file(64 * 1024);
    scrub(&mut Cursor::new(&mut buf)).unwrap();
    let rec = verify(&mut &buf[..], u64::MAX).unwrap();
    assert!(rec.comment.is_empty(), "scrub did not round-trip");

    if cfg!(debug_assertions) {
        println!("NOT JUDGED: debug_assertions are on; latency is refused");
        return;
    }

    let clock = Clock::new();
    println!(
        "clock {backend}  cycles/ns {cps:.3}  overhead {ov} cycles",
        backend = clock.backend(),
        cps = clock.cycles_per_ns(),
        ov = clock.overhead_cycles(),
    );

    for body in [1usize << 20, 16 << 20, 64 << 20] {
        report_scrub(&clock, body);
    }
    report_verify(&clock, 64 << 20);
}

fn report_scrub(clock: &Clock, body: usize) {
    let pristine = sample_file(body);
    let header = header_len(&pristine);
    let mut buf = pristine.clone();
    for _ in 0..20 {
        buf[..header].copy_from_slice(&pristine[..header]);
        let rec = scrub(&mut Cursor::new(black_box(&mut buf))).unwrap();
        black_box(rec.modified_ns);
    }

    let mut samples = Vec::with_capacity(100);
    for _ in 0..100 {
        // Restore is outside the timer. Each sample clears a live comment.
        buf[..header].copy_from_slice(&pristine[..header]);
        let ns = clock.measure_ns(|| {
            let rec = scrub(&mut Cursor::new(black_box(&mut buf))).unwrap();
            black_box(rec.modified_ns);
            black_box(buf[header - 1]);
        });
        samples.push(ns);
    }
    samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&samples, 50);
    let p99 = percentile(&samples, 99);
    let file_len = buf.len();
    println!(
        "scrub  body {body:>10}  header {header:>4}  file {file_len:>10}  p50 {p50:>10.1} ns  p99 {p99:>10.1} ns  {mb:>10.1} MB/s  body_bytes_read 0",
        mb = mb_per_s(file_len, p50),
    );

    buf[..header].copy_from_slice(&pristine[..header]);
    scrub(&mut Cursor::new(&mut buf)).unwrap();
    let rec = verify(&mut &buf[..], u64::MAX).unwrap();
    assert!(rec.comment.is_empty() && rec.tags.is_empty() && rec.extras.is_empty());
    assert_eq!(rec.modified_ns, pristine_record().modified_ns);
    assert_eq!(&buf[header..], &pristine[header..]);
}

fn report_verify(clock: &Clock, body: usize) {
    let payload = patterned(body);
    let rec = Record {
        modified_ns: 1,
        created_ns: None,
        comment: String::new(),
        tags: Vec::new(),
        extras: Vec::new(),
    };
    let mut buf = Cursor::new(Vec::new());
    write(&mut buf, &rec, &mut &payload[..]).unwrap();
    let bytes = buf.into_inner();
    for _ in 0..3 {
        black_box(verify(&mut &bytes[..], u64::MAX).unwrap().modified_ns);
    }
    let mut samples = Vec::with_capacity(20);
    for _ in 0..20 {
        let ns = clock.measure_ns(|| {
            let mut input = black_box(&bytes[..]);
            let rec = verify(&mut input, u64::MAX).unwrap();
            black_box(rec.modified_ns);
        });
        samples.push(ns);
    }
    samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&samples, 50);
    let p99 = percentile(&samples, 99);
    println!(
        "verify body {body:>10}  file {file:>10}  p50 {p50:>10.1} ns  p99 {p99:>10.1} ns  {mb:>10.1} MB/s  reads every payload byte",
        file = bytes.len(),
        mb = mb_per_s(bytes.len(), p50),
    );
}

fn pristine_record() -> Record {
    Record {
        modified_ns: 1_600_000_000_000_000_000,
        created_ns: Some(1_500_000_000_000_000_000),
        comment: "finder comment".into(),
        tags: vec!["Red".into(), "work".into()],
        extras: vec![("where_from".into(), b"https://example.com/a".to_vec())],
    }
}

fn patterned(len: usize) -> Vec<u8> {
    let mut body = vec![0u8; len];
    for (i, b) in body.iter_mut().enumerate() {
        *b = i as u8;
    }
    body
}

fn sample_file(body_len: usize) -> Vec<u8> {
    let body = vec![0xA5u8; body_len];
    let mut buf = Cursor::new(Vec::new());
    write(&mut buf, &pristine_record(), &mut &body[..]).unwrap();
    let bytes = buf.into_inner();
    assert_eq!(&bytes[0..8], b"EXMETA01");
    bytes
}

fn header_len(buf: &[u8]) -> usize {
    u32::from_le_bytes(buf[12..16].try_into().unwrap()) as usize
}

fn mb_per_s(bytes: usize, p50_ns: f64) -> f64 {
    if p50_ns <= 0.0 {
        return f64::INFINITY;
    }
    (bytes as f64 / 1_000_000.0) / (p50_ns / 1e9)
}

fn percentile(sorted_ns: &[f64], pct: usize) -> f64 {
    if sorted_ns.is_empty() {
        return 0.0;
    }
    let rank = ((pct as f64 / 100.0) * (sorted_ns.len() as f64 - 1.0)).round() as usize;
    sorted_ns[rank.min(sorted_ns.len() - 1)]
}

struct Clock {
    cycles_per_ns: f64,
    overhead_cycles: u64,
    backend: &'static str,
}

impl Clock {
    fn backend(&self) -> &'static str {
        self.backend
    }
    fn cycles_per_ns(&self) -> f64 {
        self.cycles_per_ns
    }
    fn overhead_cycles(&self) -> u64 {
        self.overhead_cycles
    }

    fn new() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            tsc::new_clock()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                cycles_per_ns: 1.0,
                overhead_cycles: 0,
                backend: "instant",
            }
        }
    }

    fn measure_ns(&self, op: impl FnOnce()) -> f64 {
        #[cfg(target_arch = "x86_64")]
        {
            let s = tsc::rd();
            op();
            let e = tsc::rd();
            let net = e.wrapping_sub(s).saturating_sub(self.overhead_cycles);
            net as f64 / self.cycles_per_ns
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let t0 = std::time::Instant::now();
            op();
            t0.elapsed().as_nanos() as f64
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod tsc {
    use super::Clock;
    use std::arch::x86_64::{_mm_lfence, _rdtsc};
    use std::time::{Duration, Instant};

    #[inline]
    pub fn rd() -> u64 {
        unsafe {
            _mm_lfence();
            let t = _rdtsc();
            _mm_lfence();
            t
        }
    }

    pub fn new_clock() -> Clock {
        let t0 = Instant::now();
        let c0 = rd();
        while t0.elapsed() < Duration::from_millis(200) {
            std::hint::spin_loop();
        }
        let c1 = rd();
        let elapsed_ns = t0.elapsed().as_nanos() as f64;
        let cycles_per_ns = (c1.wrapping_sub(c0)) as f64 / elapsed_ns;
        let mut overhead = u64::MAX;
        for _ in 0..2000 {
            let a = rd();
            let b = rd();
            overhead = overhead.min(b.wrapping_sub(a));
        }
        Clock {
            cycles_per_ns,
            overhead_cycles: overhead,
            backend: "rdtsc",
        }
    }
}
