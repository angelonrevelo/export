# Bench

Release profile (`opt-level = 3`, `lto = thin`, `codegen-units = 1`).
`cargo bench --bench throughput`. Debug builds exit 2 and are not timed.
31 measured iterations after 3 warmup. Input is 4,197,744 bytes.
MB/s is input bytes / p50. Rows under 1 µs print `n/a` because that is the timer floor.

| path | result | MB/s | p50 | p99 | allocs/MB |
|---|---|---:|---:|---:|---:|
| sha256 | hit | 286.89 | 14.63 ms | 15.57 ms | 0.00 |
| sniff | hit | n/a | 35 ns | 130 ns | 0.00 |
| sniff | miss | n/a | 28 ns | 354 ns | 0.00 |
| bmff_parse | hit | 2,057,718 | 2.04 µs | 5.28 µs | 7.15 |
| bmff_parse | miss | n/a | 52 ns | 594 ns | 0.24 |
| remux | hit | 275.95 | 15.21 ms | 19.66 ms | 13.34 |
| dedup | hit | 282.01 | 14.88 ms | 16.70 ms | 0.00 |

`bmff_parse` does not read `mdat`. Its MB/s is how fast a 4 MB file is accepted, not a payload copy rate. `remux` and `dedup` both hash the file first; the dedup hit does not parse.

## Reverted

Coalesced copy of adjacent samples versus one copy per sample, same 4 MB track (256 × 16 KiB): p50 255 µs vs 252 µs, gain −1.2%. Under a 5% noise floor. Reverted. The shipped copy reserves the known byte count and then copies each sample.
