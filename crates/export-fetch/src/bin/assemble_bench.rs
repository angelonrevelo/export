//! Release-only assembler timings. This is an in-process copy, not a download.
//! Debug builds print NOT JUDGED and exit 2.

use export_fetch::{assemble_stream, COALESCE_GAP, ByteRange, assemble_coalesced};
use std::io::{self, Write};
use std::time::Instant;

const PAYLOAD: usize = 8 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const SAMPLES: usize = 40;

fn main() {
    if cfg!(debug_assertions) {
        println!("NOT JUDGED");
        std::process::exit(2);
    }
    let src = vec![0x5au8; PAYLOAD];
    let mut sink = Sink { acc: 0, bytes: 0 };
    let mut wall_ns = Vec::with_capacity(SAMPLES);
    let mut cpu_ns = Vec::with_capacity(SAMPLES);
    let mut peak = 0u64;
    for _ in 0..SAMPLES {
        sink.bytes = 0;
        let cpu0 = cpu_ns_now();
        let wall0 = Instant::now();
        let stats = assemble_stream(&src, CHUNK, &mut sink).expect("assemble");
        let elapsed = wall0.elapsed().as_nanos() as u64;
        let cpu = cpu_ns_now().saturating_sub(cpu0);
        wall_ns.push(elapsed);
        cpu_ns.push(cpu);
        peak = peak.max(stats.peak_buffered);
        std::hint::black_box(sink.acc);
    }
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

    let mut sparse = Vec::new();
    let mut cursor = 0u64;
    while cursor + 64 < PAYLOAD as u64 {
        sparse.push(ByteRange::new(cursor, cursor + 32));
        cursor += 32 + 100;
    }
    let mut sparse_sink = Sink { acc: 0, bytes: 0 };
    let sparse_stats =
        assemble_coalesced(&src, &sparse, COALESCE_GAP, CHUNK, &mut sparse_sink).expect("sparse");

    println!("bench=assembler");
    println!("release=1");
    println!("payload_bytes={PAYLOAD}");
    println!("chunk_bytes={CHUNK}");
    println!("samples={SAMPLES}");
    println!("bytes_per_sec_p50={:.0}", bps(p50));
    println!("bytes_per_sec_p99={:.0}", bps(p99));
    println!("latency_ns_p50={p50}");
    println!("latency_ns_p99={p99}");
    println!("cpu_ns_per_mb_p50={:.0}", per_mb(cpu_p50));
    println!("cpu_ns_per_mb_p99={:.0}", per_mb(cpu_p99));
    println!("peak_buffered_bytes={peak}");
    println!("vm_hwm_kib={}", vm_hwm_kib().unwrap_or(0));
    println!("coalesce_gap={COALESCE_GAP}");
    println!("sparse_ranges={}", sparse.len());
    println!("sparse_reads={}", sparse_stats.reads);
    println!("sparse_bytes={}", sparse_stats.bytes);
    println!("sink_check={}", sink.acc);
    let _ = io::stdout().flush();
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
