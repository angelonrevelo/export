//! Release bench.
//!
//! Parse timings and the bulk local-stack GETs run on one thread. The
//! one-segment, two-segment, four-segment, eight-segment, map-and-eight-segment,
//! map-and-sixteen-segment, map-and-thirty-two-segment,
//! map-and-sixty-four-segment, map-and-one-hundred-twenty-eight-segment,
//! sixteen-segment, sixteen-range, coalesced-range, and split-range fetches
//! call the library client and use one local accept thread beside the client.
//! Loopback GETs run only here, on 127.0.0.1. They are not CDN downloads.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use export_hls::{
    check_hop, parse_media_playlist, read_http_message, resolve_redirect, write_http_get,
    ByteRange, ConnectPolicy, Error, Fetcher, Hop,
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

    match local_stack_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_two_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack two-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_four_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack four-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_eight_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack eight-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_map_and_eight_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack map-and-eight-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_map_and_sixteen_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack map-and-sixteen-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_map_and_thirty_two_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack map-and-thirty-two-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_map_and_sixty_four_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack map-and-sixty-four-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_map_and_one_hundred_twenty_eight_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack map-and-one-hundred-twenty-eight-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_sixteen_segment_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack sixteen-segment GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_sixteen_range_rows() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack sixteen-range GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_coalesced_range_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack coalesced range GET failed: {err}");
            std::process::exit(1);
        }
    }

    match local_stack_split_range_row() {
        Ok(report) => println!("{report}"),
        Err(err) => {
            eprintln!("local stack split range GET failed: {err}");
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

/// One media playlist names one cleartext segment. The segment is fetched
/// with [`Fetcher::get_segment`] on the local stack, not a CDN.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before connect. The body is raw text, not a transcode.
fn local_stack_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const SEGMENT_BYTES: usize = 64 * 1024;
    let body = vec![b's'; SEGMENT_BYTES];
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let segment_url = format!("http://127.0.0.1:{port}/seg.ts");
    if check_hop(&segment_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback segment was accepted without the bench opt-in".into());
    }
    let playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXTINF:10.0,\n{segment_url}\n#EXT-X-ENDLIST\n"
    );
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != 1 || playlist.segments[0].uri != segment_url {
        return Err("playlist did not name one cleartext segment".into());
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_cleartext_segment(listener, &body, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segment(&playlist, 0)
                .expect("cleartext segment");
            let elapsed = started.elapsed().as_nanos() as f64;
            assert_eq!(got.len(), SEGMENT_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(got.len());
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack segment GET bytes={SEGMENT_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names two cleartext segments. Both are fetched with
/// [`Fetcher::get_segments`] on one local-stack connection, not a CDN.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the sum of the two segment
/// bodies.
fn local_stack_two_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const FIRST_BYTES: usize = 64 * 1024;
    const SECOND_BYTES: usize = 64 * 1024;
    const COMBINED_BYTES: usize = FIRST_BYTES + SECOND_BYTES;
    let first = vec![b'a'; FIRST_BYTES];
    let second = vec![b'b'; SECOND_BYTES];
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let first_url = format!("http://127.0.0.1:{port}/seg0.ts");
    let second_url = format!("http://127.0.0.1:{port}/seg1.ts");
    if check_hop(&first_url, ConnectPolicy::default()) != Err(Error::Blocked)
        || check_hop(&second_url, ConnectPolicy::default()) != Err(Error::Blocked)
    {
        return Err("loopback segment was accepted without the bench opt-in".into());
    }
    let playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXTINF:10.0,\n{first_url}\n#EXTINF:10.0,\n{second_url}\n#EXT-X-ENDLIST\n"
    );
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != 2
        || playlist.segments[0].uri != first_url
        || playlist.segments[1].uri != second_url
    {
        return Err("playlist did not name two cleartext segments".into());
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_two_cleartext_segments(listener, &first, &second, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("two cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let combined: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].len(), FIRST_BYTES);
            assert_eq!(got[1].len(), SECOND_BYTES);
            assert_eq!(combined, COMBINED_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(combined);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack two-segment GET connections=1 bytes={COMBINED_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names four cleartext segments on one address. All four
/// are fetched with [`Fetcher::get_segments`] on one local-stack connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the sum of the four bodies.
fn local_stack_four_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 4;
    const COMBINED_BYTES: usize = SEGMENT_BYTES * COUNT;
    let bodies: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a' + index as u8; SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n");
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name four cleartext segments".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] {
            return Err("segment URI did not stay on the shared address".into());
        }
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_four_cleartext_segments(listener, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("four cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let combined: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(combined, COMBINED_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(combined);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack four-segment GET connections=1 bytes={COMBINED_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names eight cleartext segments on one address. All
/// eight are fetched with [`Fetcher::get_segments`] on one local-stack
/// connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the sum of the eight bodies.
fn local_stack_eight_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 8;
    const COMBINED_BYTES: usize = SEGMENT_BYTES * COUNT;
    let bodies: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a' + index as u8; SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n");
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name eight cleartext segments".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] {
            return Err("segment URI did not stay on the shared address".into());
        }
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_four_cleartext_segments(listener, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("eight cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let combined: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(combined, COMBINED_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(combined);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack eight-segment GET connections=1 bytes={COMBINED_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names an `#EXT-X-MAP` init segment and eight media
/// segments on one address. The init file and the eight media files are
/// fetched with [`Fetcher::get_segments`] on one local-stack connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the init body plus the eight
/// media bodies.
fn local_stack_map_and_eight_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const INIT_BYTES: usize = 1024;
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 8;
    const PAYLOAD_BYTES: usize = INIT_BYTES + SEGMENT_BYTES * COUNT;
    let init = vec![b'i'; INIT_BYTES];
    let media: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a' + index as u8; SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let init_url = format!("http://127.0.0.1:{port}/init.mp4");
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    if check_hop(&init_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback init was accepted without the bench opt-in".into());
    }
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MAP:URI=\"{init_url}\"\n"
    );
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name eight cleartext segments".into());
    }
    let parsed_init = playlist
        .init
        .as_ref()
        .ok_or("playlist did not name an init segment")?;
    if parsed_init.uri != init_url || parsed_init.byte_range.is_some() {
        return Err("init URI did not stay a whole-file map".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] || segment.map.as_ref() != Some(parsed_init) {
            return Err("media segment did not keep the shared init map".into());
        }
    }
    let mut paths = Vec::with_capacity(COUNT + 1);
    paths.push("/init.mp4".to_string());
    for index in 0..COUNT {
        paths.push(format!("/seg{index}.ts"));
    }
    let mut bodies = Vec::with_capacity(COUNT + 1);
    bodies.push(init);
    bodies.extend(media);
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_ordered_cleartext(listener, &paths, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("init and eight cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let payload: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT + 1);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(payload, PAYLOAD_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(payload);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack map-and-eight-segment GET connections=1 bytes={PAYLOAD_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names an `#EXT-X-MAP` init segment and sixteen media
/// segments on one address. The init file and the sixteen media files are
/// fetched with [`Fetcher::get_segments`] on one local-stack connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the init body plus the sixteen
/// media bodies.
fn local_stack_map_and_sixteen_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const INIT_BYTES: usize = 1024;
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 16;
    const PAYLOAD_BYTES: usize = INIT_BYTES + SEGMENT_BYTES * COUNT;
    let init = vec![b'i'; INIT_BYTES];
    let media: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a' + index as u8; SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let init_url = format!("http://127.0.0.1:{port}/init.mp4");
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    if check_hop(&init_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback init was accepted without the bench opt-in".into());
    }
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MAP:URI=\"{init_url}\"\n"
    );
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name sixteen cleartext segments".into());
    }
    let parsed_init = playlist
        .init
        .as_ref()
        .ok_or("playlist did not name an init segment")?;
    if parsed_init.uri != init_url || parsed_init.byte_range.is_some() {
        return Err("init URI did not stay a whole-file map".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] || segment.map.as_ref() != Some(parsed_init) {
            return Err("media segment did not keep the shared init map".into());
        }
    }
    let mut paths = Vec::with_capacity(COUNT + 1);
    paths.push("/init.mp4".to_string());
    for index in 0..COUNT {
        paths.push(format!("/seg{index}.ts"));
    }
    let mut bodies = Vec::with_capacity(COUNT + 1);
    bodies.push(init);
    bodies.extend(media);
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_ordered_cleartext(listener, &paths, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("init and sixteen cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let payload: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT + 1);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(payload, PAYLOAD_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(payload);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack map-and-sixteen-segment GET connections=1 bytes={PAYLOAD_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names an `#EXT-X-MAP` init segment and thirty-two media
/// segments on one address. The init file and the thirty-two media files are
/// fetched with [`Fetcher::get_segments`] on one local-stack connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the init body plus the
/// thirty-two media bodies.
fn local_stack_map_and_thirty_two_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const INIT_BYTES: usize = 1024;
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 32;
    const PAYLOAD_BYTES: usize = INIT_BYTES + SEGMENT_BYTES * COUNT;
    let init = vec![b'i'; INIT_BYTES];
    let media: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a'.wrapping_add(index as u8); SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let init_url = format!("http://127.0.0.1:{port}/init.mp4");
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    if check_hop(&init_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback init was accepted without the bench opt-in".into());
    }
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MAP:URI=\"{init_url}\"\n"
    );
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name thirty-two cleartext segments".into());
    }
    let parsed_init = playlist
        .init
        .as_ref()
        .ok_or("playlist did not name an init segment")?;
    if parsed_init.uri != init_url || parsed_init.byte_range.is_some() {
        return Err("init URI did not stay a whole-file map".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] || segment.map.as_ref() != Some(parsed_init) {
            return Err("media segment did not keep the shared init map".into());
        }
    }
    let mut paths = Vec::with_capacity(COUNT + 1);
    paths.push("/init.mp4".to_string());
    for index in 0..COUNT {
        paths.push(format!("/seg{index}.ts"));
    }
    let mut bodies = Vec::with_capacity(COUNT + 1);
    bodies.push(init);
    bodies.extend(media);
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_ordered_cleartext(listener, &paths, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("init and thirty-two cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let payload: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT + 1);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(payload, PAYLOAD_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(payload);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack map-and-thirty-two-segment GET connections=1 bytes={PAYLOAD_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names an `#EXT-X-MAP` init segment and sixty-four media
/// segments on one address. The init file and the sixty-four media files are
/// fetched with [`Fetcher::get_segments`] on one local-stack connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the init body plus the
/// sixty-four media bodies.
fn local_stack_map_and_sixty_four_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const INIT_BYTES: usize = 1024;
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 64;
    const PAYLOAD_BYTES: usize = INIT_BYTES + SEGMENT_BYTES * COUNT;
    let init = vec![b'i'; INIT_BYTES];
    let media: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a'.wrapping_add(index as u8); SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let init_url = format!("http://127.0.0.1:{port}/init.mp4");
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    if check_hop(&init_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback init was accepted without the bench opt-in".into());
    }
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MAP:URI=\"{init_url}\"\n"
    );
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name sixty-four cleartext segments".into());
    }
    let parsed_init = playlist
        .init
        .as_ref()
        .ok_or("playlist did not name an init segment")?;
    if parsed_init.uri != init_url || parsed_init.byte_range.is_some() {
        return Err("init URI did not stay a whole-file map".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] || segment.map.as_ref() != Some(parsed_init) {
            return Err("media segment did not keep the shared init map".into());
        }
    }
    let mut paths = Vec::with_capacity(COUNT + 1);
    paths.push("/init.mp4".to_string());
    for index in 0..COUNT {
        paths.push(format!("/seg{index}.ts"));
    }
    let mut bodies = Vec::with_capacity(COUNT + 1);
    bodies.push(init);
    bodies.extend(media);
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_ordered_cleartext(listener, &paths, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("init and sixty-four cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let payload: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT + 1);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(payload, PAYLOAD_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(payload);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack map-and-sixty-four-segment GET connections=1 bytes={PAYLOAD_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names an `#EXT-X-MAP` init segment and one hundred
/// twenty-eight media segments on one address. The init file and the media
/// files are fetched with [`Fetcher::get_segments`] on one local-stack
/// connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the init body plus the media
/// bodies.
fn local_stack_map_and_one_hundred_twenty_eight_segment_row(
) -> Result<String, Box<dyn std::error::Error>> {
    const INIT_BYTES: usize = 1024;
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 128;
    const PAYLOAD_BYTES: usize = INIT_BYTES + SEGMENT_BYTES * COUNT;
    let init = vec![b'i'; INIT_BYTES];
    let media: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a'.wrapping_add(index as u8); SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let init_url = format!("http://127.0.0.1:{port}/init.mp4");
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    if check_hop(&init_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback init was accepted without the bench opt-in".into());
    }
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MAP:URI=\"{init_url}\"\n"
    );
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name one hundred twenty-eight cleartext segments".into());
    }
    let parsed_init = playlist
        .init
        .as_ref()
        .ok_or("playlist did not name an init segment")?;
    if parsed_init.uri != init_url || parsed_init.byte_range.is_some() {
        return Err("init URI did not stay a whole-file map".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] || segment.map.as_ref() != Some(parsed_init) {
            return Err("media segment did not keep the shared init map".into());
        }
    }
    let mut paths = Vec::with_capacity(COUNT + 1);
    paths.push("/init.mp4".to_string());
    for index in 0..COUNT {
        paths.push(format!("/seg{index}.ts"));
    }
    let mut bodies = Vec::with_capacity(COUNT + 1);
    bodies.push(init);
    bodies.extend(media);
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_ordered_cleartext(listener, &paths, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("init and one hundred twenty-eight cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let payload: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT + 1);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(payload, PAYLOAD_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(payload);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack map-and-one-hundred-twenty-eight-segment GET connections=1 bytes={PAYLOAD_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One media playlist names sixteen cleartext segments on one address. All
/// sixteen are fetched with [`Fetcher::get_segments`] on one local-stack
/// connection.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode. The reported byte count is the sum of the sixteen bodies.
fn local_stack_sixteen_segment_row() -> Result<String, Box<dyn std::error::Error>> {
    const SEGMENT_BYTES: usize = 64 * 1024;
    const COUNT: usize = 16;
    const COMBINED_BYTES: usize = SEGMENT_BYTES * COUNT;
    let bodies: Vec<Vec<u8>> = (0..COUNT)
        .map(|index| vec![b'a' + index as u8; SEGMENT_BYTES])
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let urls: Vec<String> = (0..COUNT)
        .map(|index| format!("http://127.0.0.1:{port}/seg{index}.ts"))
        .collect();
    for url in &urls {
        if check_hop(url, ConnectPolicy::default()) != Err(Error::Blocked) {
            return Err("loopback segment was accepted without the bench opt-in".into());
        }
    }
    let mut playlist_text = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n");
    for url in &urls {
        playlist_text.push_str("#EXTINF:10.0,\n");
        playlist_text.push_str(url);
        playlist_text.push('\n');
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != COUNT {
        return Err("playlist did not name sixteen cleartext segments".into());
    }
    for (index, segment) in playlist.segments.iter().enumerate() {
        if segment.uri != urls[index] {
            return Err("segment URI did not stay on the shared address".into());
        }
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_four_cleartext_segments(listener, &bodies, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_segments(&playlist)
                .expect("sixteen cleartext segments");
            let elapsed = started.elapsed().as_nanos() as f64;
            let combined: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), COUNT);
            for (part, body) in got.iter().zip(bodies.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(combined, COMBINED_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(combined);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack sixteen-segment GET connections=1 bytes={COMBINED_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// Sixteen `#EXT-X-BYTERANGE` rows of the same URI, each 32768 bytes, with a
/// 4097-byte gap. They must not coalesce. The hole is not fetched. One sample
/// is all sixteen `Range` GETs, either on one connection or on sixteen.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before the connect and again on any redirect. The bodies are raw text,
/// not a transcode.
fn local_stack_sixteen_range_rows() -> Result<String, Box<dyn std::error::Error>> {
    const COUNT: usize = 16;
    const RANGE_BYTES: usize = 32 * 1024;
    const GAP_BYTES: usize = 4097;
    let mut ranges = Vec::with_capacity(COUNT);
    let mut parts = Vec::with_capacity(COUNT);
    let mut resource = Vec::new();
    for index in 0..COUNT {
        let offset = index * (RANGE_BYTES + GAP_BYTES);
        ranges.push(ByteRange {
            offset: offset as u64,
            length: RANGE_BYTES as u64,
        });
        let part = vec![b'a' + index as u8; RANGE_BYTES];
        resource.extend_from_slice(&part);
        parts.push(part);
        if index + 1 != COUNT {
            resource.extend(std::iter::repeat(b'h').take(GAP_BYTES));
        }
    }
    let separate = time_sixteen_ranges(true, &ranges, &parts, resource.len())?;
    let shared = time_sixteen_ranges(false, &ranges, &parts, resource.len())?;
    Ok(format!("{separate}\n{shared}"))
}

fn time_sixteen_ranges(
    separate_connections: bool,
    ranges: &[ByteRange],
    parts: &[Vec<u8>],
    resource_len: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    const PAYLOAD_BYTES: usize = 32 * 1024 * 16;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let segment_url = format!("http://127.0.0.1:{port}/seg.ts");
    if check_hop(&segment_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback segment was accepted without the bench opt-in".into());
    }
    let mut playlist_text = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n");
    for range in ranges {
        playlist_text.push_str(&format!(
            "#EXTINF:1.0,\n#EXT-X-BYTERANGE:{}@{}\n{segment_url}\n",
            range.length, range.offset
        ));
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.spans.len() != ranges.len() || playlist.segments.len() != ranges.len() {
        return Err("4097-byte gaps coalesced".into());
    }
    for (index, span) in playlist.spans.iter().enumerate() {
        if span.byte_range != Some(ranges[index]) || span.segments != [index] {
            return Err("a 4097-byte gap was merged".into());
        }
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let requests_per_connection = if separate_connections {
        1
    } else {
        ranges.len()
    };
    let connections = if separate_connections {
        ranges.len() * (warmup + iters)
    } else {
        warmup + iters
    };
    let connection_count = if separate_connections {
        ranges.len()
    } else {
        1
    };
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_unmerged_ranges(
                listener,
                parts,
                ranges,
                resource_len,
                requests_per_connection,
                connections,
            );
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = if separate_connections {
                let mut bodies = Vec::with_capacity(ranges.len());
                for span_index in 0..ranges.len() {
                    match fetcher.get_span(&playlist, span_index) {
                        Ok(body) => bodies.push(body),
                        Err(err) => panic!(
                            "separate cleartext range index={index} span={span_index} port={port}: {err}"
                        ),
                    }
                }
                bodies
            } else {
                match fetcher.get_spans(&playlist) {
                    Ok(bodies) => bodies,
                    Err(err) => panic!("shared cleartext ranges index={index} port={port}: {err}"),
                }
            };
            let elapsed = started.elapsed().as_nanos() as f64;
            let payload: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), ranges.len());
            for (part, body) in got.iter().zip(parts.iter()) {
                assert_eq!(part, body);
            }
            assert_eq!(payload, PAYLOAD_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(payload);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    let label = if separate_connections {
        "separate"
    } else {
        "shared"
    };
    Ok(format!(
        "local stack sixteen-range {label} GET connections={connection_count} bytes={PAYLOAD_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

fn serve_unmerged_ranges(
    listener: TcpListener,
    parts: &[Vec<u8>],
    ranges: &[ByteRange],
    resource_len: usize,
    requests_per_connection: usize,
    connections: usize,
) {
    listener.set_nonblocking(true).expect("nonblocking accept");
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut served = 0;
    let headers: Vec<String> = ranges.iter().copied().map(range_header).collect();
    while served < connections && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).expect("blocking socket");
                sock.set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                sock.set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                sock.set_nodelay(true).expect("nodelay");
                for request_index in 0..requests_per_connection {
                    let request = read_http_request(&mut sock).expect("range did not arrive");
                    assert!(
                        request.starts_with("GET /seg.ts "),
                        "range fetch used an unexpected path"
                    );
                    let last = request_index + 1 == requests_per_connection;
                    if last || requests_per_connection == 1 {
                        assert!(
                            request.contains("Connection: close\r\n"),
                            "range connection was not closed"
                        );
                    } else {
                        assert!(
                            request.contains("Connection: keep-alive\r\n"),
                            "shared range connection closed early"
                        );
                    }
                    let position = headers
                        .iter()
                        .position(|header| request.contains(header))
                        .expect("4097-byte gap was fetched as one range");
                    let body = &parts[position];
                    let range = ranges[position];
                    let inclusive_end = range.end().expect("range end") - 1;
                    let persistence = if last { "close" } else { "keep-alive" };
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{inclusive_end}/{resource_len}\r\nConnection: {persistence}\r\nContent-Type: text/plain\r\n\r\n",
                        body.len(),
                        range.offset
                    );
                    sock.write_all(header.as_bytes()).expect("range header");
                    sock.write_all(body).expect("range body");
                }
                served += 1;
            }
            Err(err) if waited(&err) => std::hint::spin_loop(),
            Err(err) => panic!("sixteen-range accept failed: {err}"),
        }
    }
    if served < connections {
        panic!(
            "sixteen-range server stopped early served={served} connections={connections} requests_per_connection={requests_per_connection}"
        );
    }
}

fn serve_ordered_cleartext(
    listener: TcpListener,
    paths: &[String],
    bodies: &[Vec<u8>],
    connections: usize,
) {
    listener.set_nonblocking(true).expect("nonblocking accept");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut served = 0;
    while served < connections && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).expect("blocking socket");
                sock.set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                sock.set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                sock.set_nodelay(true).expect("nodelay");
                for request_index in 0..bodies.len() {
                    let request = read_http_request(&mut sock)
                        .expect("later init or segment did not reuse the connection");
                    let path = format!("GET {} ", paths[request_index]);
                    assert!(
                        request.starts_with(&path),
                        "map fetch left the shared connection"
                    );
                    if request_index + 1 == bodies.len() {
                        assert!(
                            request.contains("Connection: close\r\n"),
                            "last segment did not close the shared connection"
                        );
                    } else {
                        assert!(
                            request.contains("Connection: keep-alive\r\n"),
                            "init or segment closed the shared connection early"
                        );
                    }
                    let body = &bodies[request_index];
                    let persistence = if request_index + 1 == bodies.len() {
                        "close"
                    } else {
                        "keep-alive"
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: {persistence}\r\nContent-Type: text/plain\r\n\r\n",
                        body.len()
                    );
                    sock.write_all(header.as_bytes()).expect("segment header");
                    sock.write_all(body).expect("segment body");
                }
                served += 1;
            }
            Err(err) if waited(&err) => std::hint::spin_loop(),
            Err(err) => panic!("map accept failed: {err}"),
        }
    }
    if served < connections {
        panic!("map server stopped early served={served} connections={connections}");
    }
}

fn serve_four_cleartext_segments(listener: TcpListener, bodies: &[Vec<u8>], connections: usize) {
    listener.set_nonblocking(true).expect("nonblocking accept");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut served = 0;
    while served < connections && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).expect("blocking socket");
                sock.set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                sock.set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                for request_index in 0..bodies.len() {
                    let request = read_http_request(&mut sock)
                        .expect("later segment did not reuse the connection");
                    let path = format!("GET /seg{request_index}.ts ");
                    assert!(
                        request.starts_with(&path),
                        "four-segment fetch left the shared connection"
                    );
                    if request_index + 1 == bodies.len() {
                        assert!(
                            request.contains("Connection: close\r\n"),
                            "last segment did not close the shared connection"
                        );
                    } else {
                        assert!(
                            request.contains("Connection: keep-alive\r\n"),
                            "segment closed the shared connection early"
                        );
                    }
                    let body = &bodies[request_index];
                    let persistence = if request_index + 1 == bodies.len() {
                        "close"
                    } else {
                        "keep-alive"
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: {persistence}\r\nContent-Type: text/plain\r\n\r\n",
                        body.len()
                    );
                    sock.write_all(header.as_bytes()).expect("segment header");
                    sock.write_all(body).expect("segment body");
                }
                served += 1;
            }
            Err(err) if waited(&err) => std::hint::spin_loop(),
            Err(err) => panic!("four-segment accept failed: {err}"),
        }
    }
}

/// One 65,536-byte segment is named as two adjacent byte ranges of the same
/// URI. The gap is 4096 bytes, so the client coalesces them and
/// [`Fetcher::get_span`] issues one cleartext `Range` GET. The response is
/// the merged span, which covers the hole.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before connect. The body is raw text, not a transcode.
fn local_stack_coalesced_range_row() -> Result<String, Box<dyn std::error::Error>> {
    const NAMED_BYTES: usize = 64 * 1024;
    const GAP_BYTES: usize = 4096;
    const FIRST_BYTES: usize = NAMED_BYTES / 2;
    const SECOND_BYTES: usize = NAMED_BYTES - FIRST_BYTES;
    const MERGED_BYTES: usize = FIRST_BYTES + GAP_BYTES + SECOND_BYTES;
    let second_offset = FIRST_BYTES + GAP_BYTES;
    let mut body = Vec::with_capacity(MERGED_BYTES);
    body.extend(std::iter::repeat(b'a').take(FIRST_BYTES));
    body.extend(std::iter::repeat(b'h').take(GAP_BYTES));
    body.extend(std::iter::repeat(b'b').take(SECOND_BYTES));
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let segment_url = format!("http://127.0.0.1:{port}/seg.ts");
    if check_hop(&segment_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback segment was accepted without the bench opt-in".into());
    }
    let playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXTINF:5.0,\n#EXT-X-BYTERANGE:{FIRST_BYTES}@0\n{segment_url}\n#EXTINF:5.0,\n#EXT-X-BYTERANGE:{SECOND_BYTES}@{second_offset}\n{segment_url}\n#EXT-X-ENDLIST\n"
    );
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != 2 || playlist.spans.len() != 1 {
        return Err("playlist did not coalesce two adjacent ranges".into());
    }
    if playlist.segments[0].uri != segment_url || playlist.segments[1].uri != segment_url {
        return Err("both ranges must name the same cleartext segment".into());
    }
    if playlist.segments[0].byte_range
        != Some(ByteRange {
            offset: 0,
            length: FIRST_BYTES as u64,
        })
        || playlist.segments[1].byte_range
            != Some(ByteRange {
                offset: second_offset as u64,
                length: SECOND_BYTES as u64,
            })
    {
        return Err("segment rows were rewritten before the fetch".into());
    }
    let merged = playlist.spans[0]
        .byte_range
        .ok_or("coalesced span has no range")?;
    if merged
        != (ByteRange {
            offset: 0,
            length: MERGED_BYTES as u64,
        })
        || playlist.spans[0].segments != [0, 1]
        || MERGED_BYTES <= NAMED_BYTES
    {
        return Err("merged span did not cover the hole".into());
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_coalesced_range(listener, &body, warmup + iters);
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher
                .get_span(&playlist, 0)
                .expect("coalesced cleartext range");
            let elapsed = started.elapsed().as_nanos() as f64;
            assert_eq!(got.len(), MERGED_BYTES);
            assert_eq!(got, body);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(got.len());
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack coalesced range GET bytes={MERGED_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

/// One segment is named as two ranges of the same URI with a 4097-byte gap.
/// That gap is past the coalesce limit, so [`Fetcher::get_spans`] issues two
/// cleartext `Range` GETs on one connection. The hole is not fetched. The
/// reported byte count is the sum of the two responses.
///
/// Loopback is allowed only here. `check_hop` still runs inside the client
/// before each connect. The bodies are raw text, not a transcode.
fn local_stack_split_range_row() -> Result<String, Box<dyn std::error::Error>> {
    const RANGE_BYTES: usize = 32 * 1024;
    const GAP_BYTES: usize = 4097;
    const COMBINED_BYTES: usize = RANGE_BYTES + RANGE_BYTES;
    let second_offset = RANGE_BYTES + GAP_BYTES;
    let mut resource = Vec::with_capacity(second_offset + RANGE_BYTES);
    resource.extend(std::iter::repeat(b'a').take(RANGE_BYTES));
    resource.extend(std::iter::repeat(b'h').take(GAP_BYTES));
    resource.extend(std::iter::repeat(b'b').take(RANGE_BYTES));
    let first = resource[..RANGE_BYTES].to_vec();
    let second = resource[second_offset..second_offset + RANGE_BYTES].to_vec();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let segment_url = format!("http://127.0.0.1:{port}/seg.ts");
    if check_hop(&segment_url, ConnectPolicy::default()) != Err(Error::Blocked) {
        return Err("loopback segment was accepted without the bench opt-in".into());
    }
    let playlist_text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXTINF:5.0,\n#EXT-X-BYTERANGE:{RANGE_BYTES}@0\n{segment_url}\n#EXTINF:5.0,\n#EXT-X-BYTERANGE:{RANGE_BYTES}@{second_offset}\n{segment_url}\n#EXT-X-ENDLIST\n"
    );
    let playlist = parse_media_playlist(&playlist_text)?;
    if playlist.segments.len() != 2 || playlist.spans.len() != 2 {
        return Err("a 4097-byte gap coalesced".into());
    }
    if playlist.segments[0].uri != segment_url || playlist.segments[1].uri != segment_url {
        return Err("both ranges must name the same cleartext segment".into());
    }
    let first_range = ByteRange {
        offset: 0,
        length: RANGE_BYTES as u64,
    };
    let second_range = ByteRange {
        offset: second_offset as u64,
        length: RANGE_BYTES as u64,
    };
    if playlist.spans[0].byte_range != Some(first_range)
        || playlist.spans[1].byte_range != Some(second_range)
        || playlist.spans[0].segments != [0]
        || playlist.spans[1].segments != [1]
    {
        return Err("split ranges were rewritten before the fetch".into());
    }
    let fetcher = Fetcher {
        policy: ConnectPolicy::new(true),
        timeout: Duration::from_secs(2),
        max_redirects: 5,
        max_body_bytes: 8 * 1024 * 1024,
    };
    let warmup = 5;
    let iters = 100;
    let samples = std::thread::scope(|scope| {
        scope.spawn(|| {
            serve_split_ranges(
                listener,
                &first,
                &second,
                first_range,
                second_range,
                resource.len(),
                warmup + iters,
            );
        });
        let mut samples = Vec::with_capacity(iters);
        for index in 0..(warmup + iters) {
            let started = Instant::now();
            let got = fetcher.get_spans(&playlist).expect("two cleartext ranges");
            let elapsed = started.elapsed().as_nanos() as f64;
            let combined: usize = got.iter().map(Vec::len).sum();
            assert_eq!(got.len(), 2);
            assert_eq!(got[0], first);
            assert_eq!(got[1], second);
            assert_eq!(combined, COMBINED_BYTES);
            if index >= warmup {
                samples.push(elapsed);
            }
            std::hint::black_box(combined);
        }
        samples
    });
    let mut sorted = samples;
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    Ok(format!(
        "local stack split range GET gets=2 connections=1 bytes={COMBINED_BYTES} p50_ns={p50:.0} p99_ns={p99:.0} accept_thread=1 (not a CDN download)"
    ))
}

fn serve_split_ranges(
    listener: TcpListener,
    first: &[u8],
    second: &[u8],
    first_range: ByteRange,
    second_range: ByteRange,
    resource_len: usize,
    connections: usize,
) {
    listener.set_nonblocking(true).expect("nonblocking accept");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut served = 0;
    let first_header = range_header(first_range);
    let second_header = range_header(second_range);
    while served < connections && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).expect("blocking socket");
                sock.set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                sock.set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                for request_index in 0..2 {
                    let request = read_http_request(&mut sock)
                        .expect("second range did not reuse the connection");
                    assert!(
                        request.starts_with("GET /seg.ts "),
                        "split fetch used an unexpected path"
                    );
                    if request_index == 0 {
                        assert!(
                            request.contains("Connection: keep-alive\r\n"),
                            "first range closed the connection"
                        );
                    }
                    let (body, range) = if request.contains(&first_header) {
                        (first, first_range)
                    } else if request.contains(&second_header) {
                        (second, second_range)
                    } else {
                        panic!("4097-byte gap was fetched as one range");
                    };
                    let inclusive_end = range.end().expect("range end") - 1;
                    let persistence = if request_index == 0 {
                        "keep-alive"
                    } else {
                        "close"
                    };
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{inclusive_end}/{}\r\nConnection: {persistence}\r\nContent-Type: text/plain\r\n\r\n",
                        body.len(),
                        range.offset,
                        resource_len
                    );
                    sock.write_all(header.as_bytes()).expect("range header");
                    sock.write_all(body).expect("range body");
                }
                served += 1;
            }
            Err(err) if waited(&err) => std::hint::spin_loop(),
            Err(err) => panic!("split range accept failed: {err}"),
        }
    }
}

fn range_header(range: ByteRange) -> String {
    let inclusive_end = range.end().expect("range end") - 1;
    format!("Range: bytes={}-{inclusive_end}\r\n", range.offset)
}

fn serve_coalesced_range(listener: TcpListener, body: &[u8], accepts: usize) {
    listener.set_nonblocking(true).expect("nonblocking accept");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut served = 0;
    let expected = format!("Range: bytes=0-{}\r\n", body.len() - 1);
    while served < accepts && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).expect("blocking socket");
                sock.set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                sock.set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                let request = read_http_request(&mut sock).expect("range request");
                assert!(
                    request.starts_with("GET /seg.ts "),
                    "coalesced fetch used an unexpected path"
                );
                assert!(
                    request.contains(&expected),
                    "coalesced fetch was not one Range GET"
                );
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\nContent-Type: text/plain\r\n\r\n",
                    body.len(),
                    body.len() - 1,
                    body.len()
                );
                sock.write_all(header.as_bytes()).expect("range header");
                sock.write_all(body).expect("range body");
                served += 1;
            }
            Err(err) if waited(&err) => std::hint::spin_loop(),
            Err(err) => panic!("range accept failed: {err}"),
        }
    }
}

fn serve_two_cleartext_segments(
    listener: TcpListener,
    first: &[u8],
    second: &[u8],
    connections: usize,
) {
    listener.set_nonblocking(true).expect("nonblocking accept");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut served = 0;
    while served < connections && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).expect("blocking socket");
                sock.set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                sock.set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                for request_index in 0..2 {
                    let request = read_http_request(&mut sock)
                        .expect("second segment did not reuse the connection");
                    if request_index == 0 {
                        assert!(
                            request.contains("Connection: keep-alive\r\n"),
                            "first segment closed the connection"
                        );
                    }
                    let body = if request.starts_with("GET /seg0.ts ") {
                        first
                    } else if request.starts_with("GET /seg1.ts ") {
                        second
                    } else {
                        panic!("unexpected segment path");
                    };
                    let persistence = if request_index == 0 {
                        "keep-alive"
                    } else {
                        "close"
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: {persistence}\r\nContent-Type: text/plain\r\n\r\n",
                        body.len()
                    );
                    sock.write_all(header.as_bytes()).expect("segment header");
                    sock.write_all(body).expect("segment body");
                }
                served += 1;
            }
            Err(err) if waited(&err) => std::hint::spin_loop(),
            Err(err) => panic!("two-segment accept failed: {err}"),
        }
    }
}

fn serve_cleartext_segment(listener: TcpListener, body: &[u8], accepts: usize) {
    listener.set_nonblocking(true).expect("nonblocking accept");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut served = 0;
    while served < accepts && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut sock, _)) => {
                sock.set_nonblocking(false).expect("blocking socket");
                sock.set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                sock.set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                let _ = read_http_request(&mut sock).expect("segment request");
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: text/plain\r\n\r\n",
                    body.len()
                );
                sock.write_all(header.as_bytes()).expect("segment header");
                sock.write_all(body).expect("segment body");
                served += 1;
            }
            Err(err) if waited(&err) => std::hint::spin_loop(),
            Err(err) => panic!("segment accept failed: {err}"),
        }
    }
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
