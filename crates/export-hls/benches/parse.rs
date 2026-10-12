//! Release parse bench.
//!
//! Threading is off. The loopback GET is a local stack exchange on
//! 127.0.0.1. It is not a CDN download.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use export_hls::{
    check_hop, parse_media_playlist, read_http_message, write_http_get, ConnectPolicy, Error,
};

fn main() {
    let playlist = generated_playlist(1024);
    let truncated = truncated_playlist(&playlist);
    match parse_media_playlist(&truncated) {
        Err(Error::Truncated) => {}
        other => {
            eprintln!("truncated fixture did not miss: {other:?}");
            std::process::exit(1);
        }
    }

    warmup(&playlist, &truncated);

    let ok = time_iters(200, || {
        let parsed = parse_media_playlist(&playlist).expect("generated playlist");
        std::hint::black_box(parsed.segments.len());
    });
    let miss = time_iters(200, || {
        let err = parse_media_playlist(&truncated).expect_err("truncated playlist");
        std::hint::black_box(err);
    });

    println!("clock: std::time::Instant");
    println!("threading: off");
    println!(
        "parse_ok segments=1024 bytes={} iters=200 p50_ns={:.0} p99_ns={:.0}",
        playlist.len(),
        percentile(&ok, 0.50),
        percentile(&ok, 0.99)
    );
    println!(
        "truncated_miss bytes={} iters=200 p50_ns={:.0} p99_ns={:.0}",
        truncated.len(),
        percentile(&miss, 0.50),
        percentile(&miss, 0.99)
    );

    match local_stack_get() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack loopback GET failed: {err}");
            std::process::exit(1);
        }
    }
}

fn warmup(playlist: &str, truncated: &str) {
    for _ in 0..20 {
        std::hint::black_box(
            parse_media_playlist(playlist)
                .expect("warmup")
                .segments
                .len(),
        );
        std::hint::black_box(parse_media_playlist(truncated).expect_err("warmup miss"));
    }
}

fn time_iters(iters: usize, mut op: impl FnMut()) -> Vec<f64> {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let started = Instant::now();
        op();
        samples.push(started.elapsed().as_nanos() as f64);
    }
    samples.sort_by(|left, right| left.partial_cmp(right).unwrap());
    samples
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank]
}

/// Single-threaded cleartext GET against 127.0.0.1.
///
/// Linux completes the TCP handshake into the listen backlog, so `connect`
/// returns before `accept`. No second thread is used. This is the local
/// stack, not a CDN download.
fn local_stack_get() -> Result<String, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}/playlist.m3u8");
    let hop = check_hop(&url, ConnectPolicy::new(true))?;
    let mut client = TcpStream::connect_timeout(&hop.addr, Duration::from_secs(2))?;
    client.set_read_timeout(Some(Duration::from_secs(2)))?;
    client.set_write_timeout(Some(Duration::from_secs(2)))?;
    let (mut server, _) = listener.accept()?;
    server.set_read_timeout(Some(Duration::from_secs(2)))?;
    server.set_write_timeout(Some(Duration::from_secs(2)))?;

    write_http_get(&mut client, &hop.url, None)?;
    client.flush()?;

    let mut request = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let read = server.read(&mut tmp)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&tmp[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request_text = String::from_utf8_lossy(&request);
    if request_text.to_ascii_lowercase().contains("cookie") {
        return Err("local stack request carried a cookie".into());
    }

    let body = b"#EXTM3U\n#EXTINF:1,\nseg.ts\n#EXT-X-ENDLIST\n";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/vnd.apple.mpegurl\r\n\r\n",
        body.len()
    );
    server.write_all(response.as_bytes())?;
    server.write_all(body)?;
    server.flush()?;
    drop(server);

    let message = read_http_message(&mut client, 1024 * 1024)?;
    let text = std::str::from_utf8(&message.body)?;
    let playlist = parse_media_playlist(text)?;
    Ok(format!(
        "local stack loopback GET status={} bytes={} segments={} (not a CDN download)",
        message.status,
        message.body.len(),
        playlist.segments.len()
    ))
}

fn generated_playlist(count: usize) -> String {
    let mut text = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n",
    );
    for index in 0..count {
        let offset = index as u64 * 10_000;
        text.push_str("#EXTINF:6.000,\n");
        text.push_str(&format!("#EXT-X-BYTERANGE:9000@{offset}\n"));
        text.push_str("media.ts\n");
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

fn truncated_playlist(playlist: &str) -> String {
    match playlist.rfind("#EXTINF") {
        Some(index) => {
            let mut cut = playlist[..index].to_string();
            cut.push_str("#EXTINF:6.");
            cut
        }
        None => String::new(),
    }
}
