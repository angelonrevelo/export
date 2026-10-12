# export-deflate

Raw DEFLATE, RFC 1951. Stored blocks, fixed Huffman blocks, and dynamic Huffman blocks. The same crate inflates them back. There is no zlib header, no Adler-32, and no link to zlib.

`compress` / `inflate` are the whole API that matters. `Method::Auto` (the default) writes the smallest of stored, fixed Huffman, and dynamic Huffman for each 256 KiB chunk. `Method::Fixed` and `Method::Dynamic` ask for those Huffman blocks, and write a stored block instead when the Huffman block is not smaller.

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

The same sentence cut to **4,194,304** bytes, same bench profile, 3 warmups, 80 samples. p99 is the slow end. Output is **15,225** bytes. Compress p50 **856.2** MB/s, compress p99 **794.6** MB/s. A second run was p50 862.7 and p99 575.8, same size. Both p50 samples are above 741 MB/s, and the output is smaller than the input, so this rung stays.

The same sentence cut to **8,388,608** bytes, same bench profile and sample count. Output is **30,449** bytes. Compress p50 **875.1** MB/s, compress p99 **800.0** MB/s. A second run was p50 875.1 and p99 750.9, same size. Both p50 samples are above 771 MB/s, and the output is smaller than the input, so this rung stays.

Inflating that 30,449-byte encoding on the same path returns the 8,388,608 input bytes. Inflate p50 **1716.0** MB/s, inflate p99 **1486.3** MB/s. A second run was p50 1757.5 and p99 1717.0. Both p50 samples are above 1,482 MB/s, so this inflate rung stays.

## Incompressible bytes

1,048,576 bytes from a fixed generator: `state = 0xA5A51234`, then `state = state * 1664525 + 1013904223` (mod 2^32), and the byte is `state >> 24`. Same bench profile, 3 warmups, 80 samples. p99 is the slow end of those samples.

| | MB/s |
|---|---|
| compress p50 | 45.0 |
| compress p99 | 36.6 |
| inflate p50 | 16435.6 |
| inflate p99 | 9804.4 |
| compressed size | 1048676 bytes |

A second run was compress 47.3 / 40.0 and inflate 16687.2 / 12878.3, still 1,048,676 bytes. That is the input plus 100 bytes of stored-block headers (20 blocks). The fixed and dynamic candidates were larger, so they were not written. Compress time is mostly building those rejected candidates. Inflate copies the stored bytes; the p99 gap between the two runs is the timer on a cached buffer.

## Limits

Raw DEFLATE has no checksum. A truncated block is an error. A flipped bit is an error when it breaks the block type, the stored `LEN`/`NLEN` complement, an over-subscribed or invalid Huffman code, or a length/distance that cannot be applied. A flipped bit in a stored payload, or a flip that is still some other valid symbol, inflates to different bytes and does not error. Trailing bytes after the final block are ignored, which is what Python's raw decompress does too.

The window is 32 KiB. Matches are greedy, with a 15-bit hash and a chain of 16, and only every fourth byte inside a match is hashed. There is no lazy match. The bitstream will not equal `zlib.compress`.

`inflate` buffers the whole output and stops at 64 MiB (`TooLarge`). It is not a streaming decoder and it is not a SIMD decoder. Huffman symbols are read one bit at a time. On incompressible input the stored fallback still builds those Huffman candidates before discarding them.
