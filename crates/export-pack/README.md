# export-pack

Streaming zip read and write. No dependencies.

An archive is a structured file: local headers, store-method payload, a CRC-32 of those bytes, a central directory. The writer puts that in `path.tmp` in the same directory, fsyncs, and renames onto `path`. A crash leaves the temporary file, which is not the archive. A general-purpose compressor wrapped around the finished zip is not this design.

## What works

| Format | Read | Write |
|---|---|---|
| Zip, store (method 0) | yes | yes |
| Zip, deflate (method 8) | refused | no |
| Zip64, encrypted, split | refused | no |

Entry payloads are copied in 64 KiB chunks. The central directory is the only structure kept in memory, and it is bounded by 65535 entries.

`Entry::name` is the raw central-directory name. Join it onto a directory only after `safe_name()` succeeds.

## Build

From this directory, so this `Cargo.toml` is the crate root and the release profile applies:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release --bin pack-bench
./target/release/pack-bench
```

A debug bench prints `NOT JUDGED` and does not report a speed.

## Honest limits

- Store only. Deflate, zip64, encryption, and split archives return an error.
- One archive is at most 4 GiB, one entry at most 4 GiB − 2 bytes, at most 65535 entries.
- The end-of-central-directory record must end the file. Trailing bytes are refused.
- On checksum mismatch, `copy_entry` may already have written payload bytes. Discard them. `extract_entry` writes `dest.tmp` and renames only after the CRC matches.
- Timestamps in the headers are fixed at 1980-01-01. The CRC covers payload bytes, not the header.
