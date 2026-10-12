# export-deflate

Raw DEFLATE, RFC 1951. Stored blocks, fixed Huffman blocks, and dynamic Huffman blocks. The same crate inflates them back. There is no zlib header, no Adler-32, and no link to zlib.

`compress` / `inflate` are the whole API that matters. `Method::Stored`, `Method::Fixed`, and `Method::Dynamic` force one block type. `Method::Auto` (the default) writes whichever of the three is smaller for each 256 KiB chunk.

A host can check the bytes with Python. `scripts/zlib_oracle.py` uses the stdlib zlib wrapper at `wbits=-15`. That process is not linked into this crate.

```bash
cargo test --manifest-path crates/export-deflate/Cargo.toml
cargo clippy --manifest-path crates/export-deflate/Cargo.toml --all-targets -- -D warnings
cargo build --manifest-path crates/export-deflate/Cargo.toml --lib --release --target wasm32-unknown-unknown
cargo bench --manifest-path crates/export-deflate/Cargo.toml
```

Release builds use `opt-level = 3`, `lto = "thin"`, and `codegen-units = 1`. Those keys live in this crate's `Cargo.toml` because this package is built on its own.

## Measured throughput

Fixture: `The quick brown fox jumps over the lazy dog.\n` repeated and cut to **1,048,576** bytes (`repeated_text`). Throughput is uncompressed bytes per second, where 1 MB = 10^6 bytes. Each sample is one `compress` or `inflate` call, including the output allocation. Samples are ordered by time. p50 is the median. p99 is the slow end (79th of 80), so its MB/s is lower.

`cargo bench` on this machine (`model name` from `/proc/cpuinfo`: Intel(R) Xeon(R) Processor), bench profile (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`), 3 warmups, 80 samples:

| | MB/s |
|---|---|
| compress p50 | 822.6 |
| compress p99 | 731.8 |
| inflate p50 | 1646.5 |
| inflate p99 | 1441.1 |
| compressed size | 3806 bytes |

A second run on the same binary was compress 834.2 / 752.4 and inflate 1669.4 / 1459.5, still 3806 bytes. These numbers are for this fixture only.

Hashing every byte of a match, measured the same way, was compress p50 522.7 then 547.7 MB/s and **3291** bytes. Inflate stayed in the same band (p50 1704.9 then 1766.2). The every-4th-byte insert is what the table uses: the compress gap is larger than that run-to-run spread, and the extra 515 bytes are real.

## Limits

Raw DEFLATE has no checksum. A truncated block is an error. A flipped bit is an error when it breaks the block type, the stored `LEN`/`NLEN` complement, an over-subscribed or invalid Huffman code, or a length/distance that cannot be applied. A flipped bit in a stored payload, or a flip that is still some other valid symbol, inflates to different bytes and does not error. Trailing bytes after the final block are ignored, which is what Python's raw decompress does too.

The window is 32 KiB. Matches are greedy, with a 15-bit hash and a chain of 16, and only every fourth byte inside a match is hashed. There is no lazy match. The bitstream will not equal `zlib.compress`.

`inflate` buffers the whole output and stops at 64 MiB (`TooLarge`). It is not a streaming decoder and it is not a SIMD decoder. Huffman symbols are read one bit at a time, so a large incompressible block is slower than the repeated-text number above.
