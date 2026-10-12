# One keyframe

This is not a video decoder, and these numbers are not a speedup claim. FFmpeg was not timed.

The file is `testdata/keyframe.ivf`: 66 bytes, one 22-byte VP9 payload. FFmpeg and this crate both produce the same 96-byte 4:2:0 picture. The maximum absolute sample difference is 0, so the recorded error bound is 0.

Release timing is a single thread decoding that file. Each sample is one `decode_ivf` call. 200 calls were discarded, then 2000 calls were ranked (nearest rank). On Linux 6.12.94+ x86_64 with rustc 1.83.0, profile `opt-level = 3`, thin LTO, and one codegen unit:

| run | p50 | p99 |
| --- | --- | --- |
| 1 | 386 ns | 435 ns |
| 2 | 389 ns | 436 ns |
| 3 | 389 ns | 443 ns |

The recorded pair is the last run: p50 389 ns, p99 443 ns. The same figures are in `bench/runs/keyframe.txt`.
