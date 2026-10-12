# export-hls limits

Cleartext HLS media playlists only. This crate does not download YouTube and it does not transcode. It does not call ffmpeg or yt-dlp.

- HTTP is cleartext HTTP/1.1 on `std::net`. There is no rustls, no cookie jar, and no browser `User-Agent`.
- `#EXT-X-KEY`, `https` URLs, userinfo, and cookie or credential query keys return `unsupported`. A `Set-Cookie`, `Cookie`, or `Authorization` response header does too.
- Loopback is refused unless the caller sets `allow_loopback`. Link-local is always refused. Both checks run again on every redirect, against the address that would be connected.
- Adjacent byte ranges of the same URI coalesce when the gap is at most 4096 bytes. The merged span covers the hole. Segment rows stay separate.
- `wasm32-unknown-unknown` builds the parser library. The socket fetcher is cfg-gated off that target.
- The release bench times a generated playlist, including a truncated miss. The 1,024-segment cargo bench row stays p50 198 µs. A local-stack loopback GET of a 1 MiB cleartext body stays 1,048,576 bytes, p50 830 µs, p99 976 µs, 1,263 MB/s. An 8 MiB cleartext local-stack GET is a separate row (redirect check included), reported as bytes, p50, p99, and MB/s. Neither GET is a CDN download. Threading is off. Numbers are in `bench/runs/`. They are not a comparison and not a speedup claim.
