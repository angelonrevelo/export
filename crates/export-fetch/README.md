# export-fetch

Zero-dependency cleartext fetcher for Export URL jobs. The crate builds by itself (`cargo test --manifest-path crates/export-fetch/Cargo.toml`). It does not sit in a workspace, and it does not depend on crates.io.

The hot path is HTTP/1.1 on `std::net::TcpStream`: redirects, ranged reads, a 4 KB coalesce gap, a byte-budget cache, and a stream into a writer. `User-Agent` is `export-fetch/0.1`. There is no TLS stack, no browser impersonation, and no cookie or credential store.

## What a URL can do

Cleartext `http` only. Userinfo in the URL is refused. Every dial, including redirects, is checked against the same private-network set as Export: loopback, link-local, multicast, reserved, and cloud metadata. The default config refuses loopback. Tests set `permit_loopback` and still block `10/8`, `169.254/16`, and the rest.

| Job | What this crate does |
| --- | --- |
| Direct `http` file (`mp4`, `mp3`, image) | Follows redirects, resumes with `Range`, streams to a writer. A `200` that ignores `Range` truncates the partial and rewrites the whole body. |
| Gallery page | Reads `img`, `srcset`, anchors, and `og:image`. One image is saved as a file. Several images become a stored (uncompressed) `images.zip`. |
| Watch / oEmbed page | Uses `og:video` or `og:image` when that URL is itself a cleartext file. |
| YouTube watch HTML | Keeps `formats` / `adaptiveFormats` entries that already have a direct `url`. `signatureCipher` without a URL, DRM, and `LOGIN_REQUIRED` are errors. An `https` resolve returns `Unsupported` before any dial. |

This crate downloads source bytes. It does not transcode to mp3, extract frames, or burn captions. Transcript, caption, and document jobs map onto an audio or video source the same way Export does; later crates do the conversion.

Refused: `https`, cipher-only YouTube, DRM, login walls, and any private or metadata address.

## Range cursor

`COALESCE_GAP` is 4096. A gap of that size or smaller becomes one read. `ByteCache` has an explicit byte budget (default 8 MiB). Going over the budget drops everything except the range just read. A range larger than the budget is not stored.

A server that answers `200` with the whole entity is accepted. The body is cached when it fits the budget. When it does not, the requested slice is taken from the stream and cached only if that slice fits.

A batch is collapsed per URL before apply. Fetch plus remove of the same URL ends as remove, in either order.

Downloads copy through a 64 KiB buffer. The stream test checks that the peak write stays inside that buffer.

## Benchmarks

Both bins are release-only (`opt-level = 3`, thin LTO, one codegen unit), 8 MiB payload, 40 samples, rustc 1.83.0 on 2026-10-12. Bytes per second is payload divided by wall latency. The p99 bytes/sec figure is the slow tail, so it is the lower throughput. A debug build prints `NOT JUDGED` and exits 2. `https` is unsupported and is not part of either run.

### Assembler (not a download)

`assemble_bench` times an in-process copy. It does not open a socket. Chunk size is 64 KiB. CPU time uses `CLOCK_PROCESS_CPUTIME_ID`. Log: `bench/runs/2026-10-12-assembler.txt`.

| Assembler | p50 | p99 |
| --- | ---: | ---: |
| Throughput | 29,090,243,268 B/s | 15,828,517,948 B/s |
| Wall latency | 288,365 ns | 529,968 ns |
| CPU | 36,156 ns/MB | 66,380 ns/MB |

Peak buffer held by the assembler: 65,536 bytes. Process VmHWM after that run: 12,204 KiB. A sparse pass of 63,550 ranges collapsed to 1 coalesced read (`coalesce_gap=4096`).

### Cleartext GET

`download_bench` issues a real HTTP/1.1 GET of 8 MiB through `download_to`. The server is `std::net::TcpListener` bound to `127.0.0.1`. `permit_loopback` is set only on that bench's `GetCfg`; the default job config still refuses loopback, and other private ranges stay blocked. One untimed warmup, then 40 timed samples. Process CPU includes the server thread in the same process. Log: `bench/runs/2026-10-12-cleartext-get.txt`.

| Cleartext GET | p50 | p99 |
| --- | ---: | ---: |
| Throughput | 7,708,658,103 B/s | 6,880,860,160 B/s |
| Wall latency | 1,088,206 ns | 1,219,122 ns |
| Process CPU | 233,160 ns/MB | 404,480 ns/MB |

Peak write into the sink: 65,536 bytes. VmHWM after that run: 10,632 KiB.

### Two 1 MiB GETs

`pair_bench` times two cleartext GETs, 1 MiB each, on this machine's `std::net` stack. It is not a CDN, and it does not call yt-dlp. The server listens on `127.0.0.1`. Loopback is allowed only in that bench. `https` stays unsupported. One untimed warmup of each mode, then 40 samples, alternating which mode runs first. The 8 MiB row above is a different run and is unchanged.

Shared means both bodies on one keep-alive connection (`ClearConn`). Separate means `download_to` twice, one connection per GET. The shared path stays only when its latency p50 is at least 10% under the separate p50. Log: `bench/runs/2026-10-12-cleartext-pair.txt`.

| Two 1 MiB GETs | Combined bytes | Connections | p50 | p99 |
| --- | ---: | ---: | ---: | ---: |
| Shared, kept | 2,097,152 | 1 | 350,373 ns | 502,708 ns |
| Separate | 2,097,152 | 2 | 402,202 ns | 563,464 ns |

350,373 is 12.9% under 402,202, which clears the 10% bar, so the shared connection stays.

### Four 1 MiB GETs

`quad_bench` times four cleartext GETs, 1 MiB each, on the same local `std::net` stack. It is not a CDN, and it does not call yt-dlp. The server listens on `127.0.0.1`. Loopback is allowed only in that bench. `https` stays unsupported. One untimed warmup of each mode, then 40 samples, alternating which mode runs first. The two-GET row above and the 8 MiB loopback row are different runs and are unchanged.

Shared means all four bodies on one keep-alive connection (`ClearConn`). Separate means `download_to` four times, one connection per GET. The shared path stays only when its latency p50 is at least 10% under the separate p50. Log: `bench/runs/2026-10-12-cleartext-quad.txt`.

| Four 1 MiB GETs | Combined bytes | Connections | p50 | p99 |
| --- | ---: | ---: | ---: | ---: |
| Shared, kept | 4,194,304 | 1 | 651,206 ns | 970,711 ns |
| Separate | 4,194,304 | 4 | 766,616 ns | 1,008,880 ns |

651,206 is 15.1% under 766,616, which clears the 10% bar, so the shared connection stays.

### Eight 1 MiB GETs

`octet_bench` times eight cleartext GETs, 1 MiB each, on the same local `std::net` stack. It is not a CDN, and it does not call yt-dlp. The server listens on `127.0.0.1`. Loopback is allowed only in that bench. `https` stays unsupported. One untimed warmup of each mode, then 40 samples, alternating which mode runs first. The four-GET row above and the two-GET row are different runs and are unchanged.

Shared means all eight bodies on one keep-alive connection (`ClearConn`). Separate means `download_to` eight times, one connection per GET. The shared path stays only when its latency p50 is at least 10% under the separate p50. Log: `bench/runs/2026-10-12-cleartext-octet.txt`.

| Eight 1 MiB GETs | Combined bytes | Connections | p50 | p99 |
| --- | ---: | ---: | ---: | ---: |
| Shared, kept | 8,388,608 | 1 | 1,221,592 ns | 1,590,166 ns |
| Separate | 8,388,608 | 8 | 1,468,142 ns | 1,969,406 ns |

1,221,592 is 16.8% under 1,468,142, which clears the 10% bar, so the shared connection stays.

```
cargo run --release --manifest-path crates/export-fetch/Cargo.toml --bin assemble_bench
cargo run --release --manifest-path crates/export-fetch/Cargo.toml --bin download_bench
cargo run --release --manifest-path crates/export-fetch/Cargo.toml --bin pair_bench
cargo run --release --manifest-path crates/export-fetch/Cargo.toml --bin quad_bench
cargo run --release --manifest-path crates/export-fetch/Cargo.toml --bin octet_bench
```
