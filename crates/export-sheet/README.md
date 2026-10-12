# export-sheet

Streaming conversions for a rectangle of cells: CSV, TSV, and one JSON array.
No dependencies. The parsers are in this crate.

```sh
cargo test --manifest-path crates/export-sheet/Cargo.toml
cargo run --release --manifest-path crates/export-sheet/Cargo.toml --bin export-sheet-bench
cargo build --release --lib --manifest-path crates/export-sheet/Cargo.toml --target wasm32-unknown-unknown
```

Run the bench from the repository root, or pass `--manifest-path` as above.
`rows_per_sec` divides by 100 000 data rows. `allocs` counts `alloc` and
`realloc` during that one conversion, including growth of the output buffer,
and not the construction of the input. The figure below is the best of three
release runs.

| conversion | rows/sec | allocs per 100 000 data rows |
|---|---:|---:|
| csv → json | 6,600,252 | 25 |
| json → csv | 2,129,244 | 26 |
| csv → tsv | 12,190,973 | 22 |

Machine: 4-thread Intel Xeon, Linux 6.12.94+. Four columns, 100 000 data rows.
`json → csv` writes 100 001 lines because the header is a line. Allocations do
not grow with the row count; a per-row `String` would show up here as hundreds
of thousands. `cargo build --release --lib --target wasm32-unknown-unknown`
produces `libexport_sheet.rlib` for the same parsers. XLSX is not in this crate
yet.

## XLS

One generated BIFF8 `.xls`: a version-3 compound file, a `Workbook` stream, and
one worksheet. Cell A1 is the number **42**. Cell B1 is the text **cat**.
`xls → csv` prints `42,cat`. A file that is not BIFF is refused. `CONTINUE`,
rich shared strings, and encryption are refused rather than guessed.

Release build, 10 000 reads of that file after 200 warmup reads, nearest rank:

| | time |
|---|---:|
| p50 | 520 ns |
| p99 | 1 147 ns |

Machine: 4-thread Intel Xeon, Linux 6.12.94+.

## What a record is

CSV and TSV use the same quoting rules. A field that contains the delimiter, a
quote, or a newline is wrapped in double quotes, and a quote inside a field is
doubled. A tab-separated file that never quotes still splits on tabs. Lines
that are empty or only spaces are skipped. A line of only tabs is a TSV row of
empty cells and is kept.

The first delimited row is the header when converting to JSON (the default).
Header cells are trimmed. Duplicate names are an error. A later row with a
different column count is an error: shifting cells under the wrong names is
worse than refusing the file. `Options { header: false }` writes a JSON array
of arrays instead, and that form is also ragged-checked against the first row.

JSON input is one array. Each element is an object of scalars or an array of
scalars. The first element decides which, and mixing them is an error. Object
keys come from the first object, in that order. A later object may omit a key
(empty cell). A key that was not on the first object is an error. Nested
objects and arrays are an error. Numbers and bools stay literal text. `null`
is an empty cell, not the word `null`. CSV has no types, so a number that
round-trips through CSV comes back as a JSON string.

Invalid UTF-8 is replaced with U+FFFD and counted on `Stats::lossy`, per
record that needed a repair. The rest of the file is still converted. A record
bigger than 16 MiB is refused, which is what keeps one unclosed quote from
reading the whole stream into one buffer.

Bytes are not preserved. Record separators are `\n`. A UTF-8 BOM at the start
is ignored. The conversion is streaming: one record, plus a fixed read window,
not the table.
