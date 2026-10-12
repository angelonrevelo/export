# export-webm

Stream copy for WebM and Matroska.

This crate copies block payloads and timestamps. It is a stream copy, not a transcode, and not a video shrink.

Each input is hashed once with SHA-256, deduped by that hash, sniffed as EBML, then copied. VP8, VP9, Opus, and Vorbis are sniffed and return `unsupported`. They are not decoded.
