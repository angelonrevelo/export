# export-meta

Portable metadata record. No dependencies.

The record stores a modified time, a created time when there is one, a
comment, tags, and extra fields in front of the file bytes. `scrub` clears
the comment, the tags, and the extra fields by rewriting that header. The
payload offset does not move, so the payload is not read.

Magic is `EXMETA01`. The head is 80 bytes. Checksums are CRC-32/ISO-HDLC.
A short file, a bad magic, a checksum miss, or a span that does not tile the
header is an error. It is not an empty record.

## Honest limits

- CRC-32 catches accidental truncation and bit-flips. It is not an authenticator.
- `scrub` checks the header checksum and the file length. It does not read the
  payload. A bit flipped in the payload still returns from `scrub`; the next
  `read` or `verify` returns a checksum error. Header-scrub throughput is file
  bytes covered per second of a header edit, not bytes read from the payload.
- Empty comment and a missing comment are the same bytes. Created time at the
  Unix epoch is `Some(0)`, which is not the same as `None`.
- Tags are opaque UTF-8. This crate does not write colour indexes or xattrs.
- Times are signed Unix nanoseconds. No timezone and no leap seconds.
- `scrub` is one header write. A crash can tear it. The next open fails the
  checksum. There is no journal.
- `write_path` renames a temporary file in the same directory into place.
  Two writers of one path share that temporary name.
- The record starts at byte 0. No older magic is recognised.

## Bench

Release only. A debug build prints `NOT JUDGED` and does not publish a rate.

```sh
cargo bench --manifest-path crates/export-meta/Cargo.toml --bench scrub
```

The measurement is the p50, not the MB/s. `scrub` time stays under a
microsecond while the payload grows 64×, because the payload is not read.
Dividing the whole file length by that time produces a huge MB/s. That
number is bytes covered, not bytes read. `verify` on the same 64 MiB
payload, which does read every byte, is about 440 MB/s.

p50 of 100 scrubs and 20 verifies. rdtsc, overhead subtracted. Two runs of
the same release binary, back to back. The table is the second run. The
first run's scrub p50s were 764.2 ns, 730.8 ns, 692.5 ns, and its verify
was 437.5 MB/s.

| op | body | header | p50 | p99 | MB/s | bytes read |
|---|---|---|---|---|---|---|
| scrub | 1,048,576 | 142 | 791.7 ns | 809.2 ns | 1,324,690.4 | header only |
| scrub | 16,777,216 | 142 | 725.8 ns | 764.2 ns | 23,114,509.8 | header only |
| scrub | 67,108,864 | 142 | 675.8 ns | 744.2 ns | 99,297,707.5 | header only |
| verify | 67,108,864 | — | 152,788,616.0 ns | 161,283,951.3 ns | 439.2 | every payload byte |

Machine: Linux 6.12.94+, 4-core Intel Xeon, rustc 1.83.0, clock rdtsc at
2.400 cycles/ns, overhead 46 cycles. Single-threaded. `opt-level = 3`,
thin LTO, `codegen-units = 1`. Recorded 2026-10-12. The bench reloads the
scrubbed file and checks the payload bytes before it prints a row.

## OOXML decrypt

`decrypt_ooxml` reads an encrypted OOXML package (the Compound File around
a docx, xlsx, or pptx) and writes the zip. The password is checked against
the encrypted verifier first. A mismatch returns `wrong password` and does
not write.

Standard encryption is minor version 2: AES-128-ECB, SHA-1, 50,000
iterations. Agile encryption is version 4.4, which current Word writes.
This crate accepts agile only for AES-256-CBC and SHA-512. Key derivation
is locked to the published `Password1234_` vectors (package key
`40206609…2ac2`, and the verifier that hashes to itself). Other agile
ciphers are refused, and that refusal is not a wrong password.

`encrypt_ooxml` and `encrypt_ooxml_agile` build fixtures in this crate.
The agile fixture uses a fixed verifier and a fixed package key. The tests
use a stored zip whose document part contains `hello-ooxml`. That fixture
is not a Word-validated docx. The container is what is tested.

### Honest limits

- Standard path: one ECB key for the package, no payload HMAC. A flipped
  standard ciphertext still decrypts, to different bytes. No AES-NI.
- Agile path: spin count at most 1,000,000. The payload HMAC is checked
  before any plaintext byte is written. A flipped agile ciphertext returns
  corrupt and writes nothing.
- Compound File version 3, 512-byte sectors. A 4096-byte sector file is
  refused.
- The fixture writers hold the ciphertext. `decrypt_ooxml` does not hold
  the plaintext. On the standard path, a truncated package after a correct
  password can write a prefix.
- `EncryptionInfo` is refused above 64 KiB. The package stream is not loaded.

### Bench

```sh
cargo bench --manifest-path crates/export-meta/Cargo.toml --bench ooxml
```

Median of 5 timed decrypts after one warmup. `Instant`, not rdtsc: these
times are milliseconds. `peak extra` is bytes requested from the allocator
during decrypt, above the ciphertext and the expected plaintext, which
were already allocated. It is not process RSS. The decrypt checks every
output byte against that plaintext.

The 64 KiB row is mostly the password stretching. The 1 MiB row is closer
to software AES. Peak extra does not grow with the package. Agile time
includes a HMAC pass over the ciphertext before plaintext is written.

Standard, recorded with the first release run:

| plaintext | file | median | MB/s | peak extra |
|---|---|---|---|---|
| 65,536 | 69,120 | 9.115 ms | 7.19 | 1,832 |
| 1,048,576 | 1,059,840 | 21.573 ms | 48.61 | 1,832 |

Agile, same bench and the same machine, recorded 2026-10-12:

| plaintext | file | median | MB/s | peak extra |
|---|---|---|---|---|
| 65,536 | 70,144 | 32.636 ms | 2.01 | 2,819 |
| 1,048,576 | 1,060,864 | 54.381 ms | 19.28 | 2,819 |

Same password and AES-256-CBC SHA-512 path, with four times the 1 MiB
plaintext. 100 timed decrypts after one warmup. Every sample matched the
plaintext. p50 and p99 are those times; MB/s divides the plaintext length
by that time. Recorded 2026-10-12.

| plaintext | file | p50 | MB/s p50 | MB/s p99 |
|---|---|---|---|---|
| 4,194,304 | 4,231,168 | 114.998 ms | 36.47 | 28.33 |

Same release profile as the scrub table. The bench aborts if peak extra
reaches 64 KiB, and aborts this size when MB/s p50 is below 17.4. A later
rerun of the standard rows on the agile bench binary stayed at 1,832 extra
bytes.
