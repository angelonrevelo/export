# One 2048×2048 keyframe

This is not a video decoder, and these numbers are not a speedup claim. FFmpeg was not timed.

The file is `testdata/keyframe-2048.ivf`: 24735 bytes, one 24691-byte VP9 payload. It is a single intra frame made of one thousand twenty-four 64×64 superblocks. Four 8×8 blocks carry one luma coefficient each; the other 65532 are skipped. FFmpeg and this crate both produce the same 6291456-byte 4:2:0 picture. The maximum absolute sample difference is 0, so the recorded error bound is 0.

Release timing is a single thread decoding that file. Each sample is one `decode_ivf` call. 200 calls were discarded, then 2000 calls were ranked (nearest rank). On Linux 6.12.94+ x86_64 with rustc 1.83.0, profile `opt-level = 3`, thin LTO, and one codegen unit:

| run | p50 | p99 |
| --- | --- | --- |
| 1 | 4789057 ns | 5178245 ns |
| 2 | 4799368 ns | 5371400 ns |
| 3 | 4802991 ns | 8856052 ns |

The recorded pair is the last run: p50 4802991 ns, p99 8856052 ns. That p50 is under 5,057 µs. The same figures are in `bench/runs/keyframe-2048.txt`.

The 8×8 row is unchanged: 66-byte file, 22-byte payload, p50 389 ns, p99 443 ns, error 0. The 16×16 row is unchanged: 74-byte file, 30-byte payload, p50 1065 ns, p99 1165 ns, error 0. The 32×32 row is unchanged: 81-byte file, 37-byte payload, p50 1870 ns, p99 3583 ns, error 0. The 64×64 row is unchanged: 100-byte file, 56-byte payload, p50 5051 ns, p99 7457 ns, error 0. The 128×128 row is unchanged: 174-byte file, 130-byte payload, p50 19052 ns, p99 24823 ns, error 0. The 256×256 row is unchanged: 466-byte file, 422-byte payload, p50 71979 ns, p99 91553 ns, error 0. The 512×512 row is unchanged: 1628-byte file, 1584-byte payload, p50 285384 ns, p99 336523 ns, error 0. The 1024×1024 row is unchanged: 6256-byte file, 6212-byte payload, p50 1149243 ns, p99 1317456 ns, error 0.
