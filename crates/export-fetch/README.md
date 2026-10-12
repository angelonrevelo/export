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

## Assembler bench

`assemble_bench` times an in-memory copy, not a socket. Payload is 8 MiB, chunk is 64 KiB, 40 samples, release profile (`opt-level = 3`, thin LTO, one codegen unit). Bytes per second is payload divided by wall latency. The p99 bytes/sec figure is the slow tail, so it is the lower throughput. CPU time uses `CLOCK_PROCESS_CPUTIME_ID`. A debug build prints `NOT JUDGED` and exits 2.

Recorded on rustc 1.83.0, 2026-10-12 (`bench/runs/2026-10-12-assembler.txt`):

| | p50 | p99 |
| --- | ---: | ---: |
| Throughput | 29,090,243,268 B/s | 15,828,517,948 B/s |
| Wall latency | 288,365 ns | 529,968 ns |
| CPU | 36,156 ns/MB | 66,380 ns/MB |

Peak buffer held by the assembler: 65,536 bytes. Process VmHWM after the run: 12,204 KiB. A sparse pass of 63,550 ranges collapsed to 1 coalesced read (`coalesce_gap=4096`).

```
cargo run --release --manifest-path crates/export-fetch/Cargo.toml --bin assemble_bench
```
