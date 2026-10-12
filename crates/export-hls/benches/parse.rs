//! Release bench.
//!
//! Threading is off. Loopback GETs run only here, on the local stack
//! (127.0.0.1). They are not CDN downloads.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use export_hls::{
    check_hop, parse_media_playlist, read_http_message, resolve_redirect, write_http_get,
    ConnectPolicy, Error, Hop,
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

    match local_stack_body_row(1024 * 1024) {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack 1 MiB loopback GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_body_row(8 * 1024 * 1024) {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack 8 MiB loopback GET failed: {err}");
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
    let request_text = read_http_request(&mut server)?;
    if !request_text.starts_with("GET /playlist.m3u8 ") {
        return Err("local stack request path mismatch".into());
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

/// Cleartext body fetched on the local stack through one redirect.
///
/// `check_hop` runs on the first URL and again on the redirect target.
/// Loopback is allowed only because this bench sets `allow_loopback`.
/// The body is raw text, not a transcode. MB/s uses 10^6 bytes and the p50
/// duration of the whole GET (redirect included).
fn local_stack_body_row(body_bytes: usize) -> Result<String, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let start_url = format!("http://127.0.0.1:{port}/start");
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/body\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let payload = vec![b'a'; body_bytes];
    let mut body_response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {body_bytes}\r\nConnection: close\r\nContent-Type: text/plain\r\n\r\n"
    )
    .into_bytes();
    body_response.extend_from_slice(&payload);
    let max_body = body_bytes.max(2 * 1024 * 1024);

    let blocked = check_hop(&start_url, ConnectPolicy::default());
    if blocked != Err(Error::Blocked) {
        return Err("loopback was accepted without the bench opt-in".into());
    }

    for _ in 0..5 {
        let n = one_cleartext_get(
            &listener,
            &start_url,
            redirect.as_bytes(),
            &body_response,
            max_body,
        )?;
        if n != body_bytes {
            return Err(format!("warmup body was {n} bytes").into());
        }
    }

    let samples = time_iters(100, || {
        let n = one_cleartext_get(
            &listener,
            &start_url,
            redirect.as_bytes(),
            &body_response,
            max_body,
        )
        .expect("local stack body");
        std::hint::black_box(n);
    });
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);
    let mb_s = megabytes_per_second(body_bytes, p50);
    Ok(format!(
        "local stack loopback GET bytes={body_bytes} p50_ns={p50:.0} p99_ns={p99:.0} MB/s={mb_s:.1} (not a CDN download)"
    ))
}

fn one_cleartext_get(
    listener: &TcpListener,
    start_url: &str,
    redirect: &[u8],
    body_response: &[u8],
    max_body: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let policy = ConnectPolicy::new(true);
    let first = check_hop(start_url, policy)?;
    let redirected = exchange(listener, &first, redirect, "GET /start ", max_body)?;
    if redirected.status != 302 {
        return Err(format!("expected redirect, got {}", redirected.status).into());
    }
    let location = redirected.location.ok_or("redirect missing location")?;
    let next = resolve_redirect(&first.url, &location)?;
    let second = check_hop(&next, policy)?;
    if !second.addr.ip().is_loopback() {
        return Err("redirect target was not loopback".into());
    }
    let message = exchange(listener, &second, body_response, "GET /body ", max_body)?;
    if message.status != 200 {
        return Err(format!("expected 200, got {}", message.status).into());
    }
    Ok(message.body.len())
}

fn exchange(
    listener: &TcpListener,
    hop: &Hop,
    response: &[u8],
    expect_request_prefix: &str,
    max_body: usize,
) -> Result<export_hls::HttpMessage, Box<dyn std::error::Error>> {
    let mut client = TcpStream::connect_timeout(&hop.addr, Duration::from_secs(2))?;
    client.set_read_timeout(Some(Duration::from_secs(2)))?;
    client.set_write_timeout(Some(Duration::from_secs(2)))?;
    let (mut server, _) = listener.accept()?;
    server.set_read_timeout(Some(Duration::from_secs(2)))?;
    server.set_write_timeout(Some(Duration::from_secs(2)))?;

    write_http_get(&mut client, &hop.url, None)?;
    client.flush()?;
    let request = read_http_request(&mut server)?;
    if !request.starts_with(expect_request_prefix) {
        return Err("local stack request path mismatch".into());
    }

    client.set_nonblocking(true)?;
    server.set_nonblocking(true)?;
    let received = pump(&mut server, &mut client, response)?;
    Ok(read_http_message(
        &mut std::io::Cursor::new(received),
        max_body,
    )?)
}

fn pump(
    server: &mut TcpStream,
    client: &mut TcpStream,
    response: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut sent = 0;
    let mut received = Vec::with_capacity(response.len());
    let mut tmp = [0u8; 64 * 1024];
    let deadline = Instant::now() + Duration::from_secs(2);
    while received.len() < response.len() {
        if Instant::now() > deadline {
            return Err("local stack transfer stalled".into());
        }
        if sent < response.len() {
            match server.write(&response[sent..]) {
                Ok(0) => return Err("server closed before the body was sent".into()),
                Ok(n) => sent += n,
                Err(err) if waited(&err) => {}
                Err(err) => return Err(err.into()),
            }
        }
        match client.read(&mut tmp) {
            Ok(0) => return Err("client closed before the body arrived".into()),
            Ok(n) => received.extend_from_slice(&tmp[..n]),
            Err(err) if waited(&err) => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(received)
}

fn waited(err: &std::io::Error) -> bool {
    matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted)
}

fn read_http_request(server: &mut TcpStream) -> Result<String, Box<dyn std::error::Error>> {
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
        if request.len() > 64 * 1024 {
            return Err("request too large".into());
        }
    }
    let text = String::from_utf8(request)?;
    if text.to_ascii_lowercase().contains("cookie")
        || text.to_ascii_lowercase().contains("authorization")
    {
        return Err("local stack request carried credentials".into());
    }
    Ok(text)
}

fn megabytes_per_second(bytes: usize, p50_ns: f64) -> f64 {
    if p50_ns <= 0.0 {
        return 0.0;
    }
    (bytes as f64) / 1_000_000.0 / (p50_ns / 1e9)
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
