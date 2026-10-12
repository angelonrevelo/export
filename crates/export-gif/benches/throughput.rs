//! Release timings. `cargo bench` uses the bench profile (lto thin, codegen-units 1).
//!
//! Pixel MB/s is indexed bytes per second at the p50. The decode bar was chosen
//! before this run: 80 pixel MB/s. A miss is printed. Video-to-GIF is a miss
//! on purpose: the call errors and produces no frames.

use std::time::Instant;

use export_gif::{decode_gif, encode_gif, video_to_gif, Frame, Gif, Version};

/// Fixed before measuring. Not a result.
const DECODE_PIXEL_MB_S_BAR: f64 = 80.0;

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    let i = rank.clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn time_n(n: usize, mut f: impl FnMut()) -> (u128, u128) {
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        f();
        samples.push(t.elapsed().as_nanos());
    }
    samples.sort_unstable();
    (percentile(&samples, 0.50), percentile(&samples, 0.99))
}

fn mb_s(bytes: f64, nanos: u128) -> f64 {
    if nanos == 0 {
        return 0.0;
    }
    (bytes / 1_000_000.0) / (nanos as f64 / 1_000_000_000.0)
}

fn fixture() -> Gif {
    let w = 128u32;
    let h = 96u32;
    let mut frames = Vec::new();
    let delays = [4u16, 8, 15, 30];
    for (i, delay) in delays.into_iter().enumerate() {
        let mut indices = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = (x.wrapping_mul(3 + i as u32) + y.wrapping_mul(5)) & 0x0f;
                indices.push(v as u8);
            }
        }
        let local = if i == 1 {
            Some((0..16).map(|c| [c * 15, 255 - c * 10, 40]).collect())
        } else {
            None
        };
        frames.push(Frame {
            left: 0,
            top: 0,
            width: w as u16,
            height: h as u16,
            delay_cs: delay,
            disposal: (i as u8) & 0x07,
            transparent: if i == 2 { Some(0) } else { None },
            local_table: local,
            indices,
        });
    }
    let global: Vec<[u8; 3]> = (0..16).map(|c| [c * 16, c * 8, 255 - c * 16]).collect();
    Gif {
        version: Version::Gif89a,
        width: w as u16,
        height: h as u16,
        background: 0,
        aspect: 0,
        global_table: Some(global),
        loop_count: Some(0),
        frames,
    }
}

fn main() {
    let gif = fixture();
    let pixels: usize = gif.frames.iter().map(|f| f.indices.len()).sum();
    let encoded = encode_gif(&gif).unwrap();
    let back = decode_gif(&encoded).unwrap();
    assert_eq!(back.frames.len(), 4);
    for (got, want) in back.frames.iter().zip(&gif.frames) {
        assert_eq!(got.indices, want.indices);
        assert_eq!(got.delay_cs, want.delay_cs);
    }

    for _ in 0..4 {
        let _ = decode_gif(&encoded).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        let _ = decode_gif(&encoded).unwrap();
    });
    let decode_mb = mb_s(pixels as f64, p50);
    let decode_mark = if decode_mb >= DECODE_PIXEL_MB_S_BAR {
        "HIT"
    } else {
        "MISS"
    };
    println!(
        "gif_decode frames=4 pixels={pixels} file_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} bar_mb_s={DECODE_PIXEL_MB_S_BAR} {decode_mark}",
        encoded.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        decode_mb,
        mb_s(pixels as f64, p99),
    );

    for _ in 0..2 {
        let _ = encode_gif(&gif).unwrap();
    }
    let (p50, p99) = time_n(20, || {
        let _ = encode_gif(&gif).unwrap();
    });
    println!(
        "gif_encode frames=4 pixels={pixels} samples=20 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixels as f64, p50),
        mb_s(pixels as f64, p99),
    );

    let mp4 = [
        0u8, 0, 0, 0x18, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2',
    ];
    for _ in 0..8 {
        let _ = video_to_gif(&mp4);
    }
    let (p50, p99) = time_n(80, || {
        let err = video_to_gif(&mp4).unwrap_err();
        assert_eq!(err.message(), "video to gif is unsupported");
    });
    println!(
        "video_to_gif MISS samples=80 p50_us={:.3} p99_us={:.3} pixels=0 outcome=unsupported",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
    );
}
