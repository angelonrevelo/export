# One 128×128 keyframe

This is not a video decoder, and these numbers are not a speedup claim. FFmpeg was not timed.

The file is `testdata/keyframe-128.ivf`: 174 bytes, one 130-byte VP9 payload. It is a single intra frame made of four 64×64 superblocks. Four 8×8 blocks carry one luma coefficient each; the other 252 are skipped. FFmpeg and this crate both produce the same 24576-byte 4:2:0 picture. The maximum absolute sample difference is 0, so the recorded error bound is 0.

Release timing is a single thread decoding that file. Each sample is one `decode_ivf` call. 200 calls were discarded, then 2000 calls were ranked (nearest rank). On Linux 6.12.94+ x86_64 with rustc 1.83.0, profile `opt-level = 3`, thin LTO, and one codegen unit:

| run | p50 | p99 |
| --- | --- | --- |
| 1 | 18987 ns | 26436 ns |
| 2 | 18956 ns | 25863 ns |
| 3 | 19052 ns | 24823 ns |

The recorded pair is the last run: p50 19052 ns, p99 24823 ns. That p50 is under 22.2 µs. The same figures are in `bench/runs/keyframe-128.txt`.

The 8×8 row is unchanged: 66-byte file, 22-byte payload, p50 389 ns, p99 443 ns, error 0. The 16×16 row is unchanged: 74-byte file, 30-byte payload, p50 1065 ns, p99 1165 ns, error 0. The 32×32 row is unchanged: 81-byte file, 37-byte payload, p50 1870 ns, p99 3583 ns, error 0. The 64×64 row is unchanged: 100-byte file, 56-byte payload, p50 5051 ns, p99 7457 ns, error 0.
