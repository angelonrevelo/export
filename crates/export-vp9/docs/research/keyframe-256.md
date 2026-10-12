# One 256×256 keyframe

This is not a video decoder, and these numbers are not a speedup claim. FFmpeg was not timed.

The file is `testdata/keyframe-256.ivf`: 466 bytes, one 422-byte VP9 payload. It is a single intra frame made of sixteen 64×64 superblocks. Four 8×8 blocks carry one luma coefficient each; the other 1020 are skipped. FFmpeg and this crate both produce the same 98304-byte 4:2:0 picture. The maximum absolute sample difference is 0, so the recorded error bound is 0.

Release timing is a single thread decoding that file. Each sample is one `decode_ivf` call. 200 calls were discarded, then 2000 calls were ranked (nearest rank). On Linux 6.12.94+ x86_64 with rustc 1.83.0, profile `opt-level = 3`, thin LTO, and one codegen unit:

| run | p50 | p99 |
| --- | --- | --- |
| 1 | 72087 ns | 91731 ns |
| 2 | 72057 ns | 92964 ns |
| 3 | 71979 ns | 91553 ns |

The recorded pair is the last run: p50 71979 ns, p99 91553 ns. That p50 is under 83.8 µs. The same figures are in `bench/runs/keyframe-256.txt`.

The 8×8 row is unchanged: 66-byte file, 22-byte payload, p50 389 ns, p99 443 ns, error 0. The 16×16 row is unchanged: 74-byte file, 30-byte payload, p50 1065 ns, p99 1165 ns, error 0. The 32×32 row is unchanged: 81-byte file, 37-byte payload, p50 1870 ns, p99 3583 ns, error 0. The 64×64 row is unchanged: 100-byte file, 56-byte payload, p50 5051 ns, p99 7457 ns, error 0. The 128×128 row is unchanged: 174-byte file, 130-byte payload, p50 19052 ns, p99 24823 ns, error 0.
