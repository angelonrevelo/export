# One 32×32 keyframe

This is not a video decoder, and these numbers are not a speedup claim. FFmpeg was not timed.

The file is `testdata/keyframe-32.ivf`: 81 bytes, one 37-byte VP9 payload. It is a single intra frame. Four 8×8 blocks carry one luma coefficient each; the other twelve are skipped. FFmpeg and this crate both produce the same 1536-byte 4:2:0 picture. The maximum absolute sample difference is 0, so the recorded error bound is 0.

Release timing is a single thread decoding that file. Each sample is one `decode_ivf` call. 200 calls were discarded, then 2000 calls were ranked (nearest rank). On Linux 6.12.94+ x86_64 with rustc 1.83.0, profile `opt-level = 3`, thin LTO, and one codegen unit:

| run | p50 | p99 |
| --- | --- | --- |
| 1 | 1843 ns | 2968 ns |
| 2 | 1918 ns | 2166 ns |
| 3 | 1870 ns | 3583 ns |

The recorded pair is the last run: p50 1870 ns, p99 3583 ns. That p50 is under 4.7 µs. The same figures are in `bench/runs/keyframe-32.txt`.

The 8×8 row is unchanged: 66-byte file, 22-byte payload, p50 389 ns, p99 443 ns, error 0. The 16×16 row is unchanged: 74-byte file, 30-byte payload, p50 1065 ns, p99 1165 ns, error 0.
