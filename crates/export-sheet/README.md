# export-sheet

Streaming conversions for a rectangle of cells: CSV, TSV, one JSON array, and
one XLSX worksheet. No dependencies. The parsers are in this crate.

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
| csv → json | 6,574,446 | 25 |
| json → csv | 2,124,207 | 26 |
| csv → tsv | 11,743,930 | 22 |
| csv → xlsx | 2,245,181 | 55 |

Machine: 4-thread Intel Xeon, Linux 6.12.94+. Four columns, 100 000 data rows.
Best of three release runs, measured together. `json → csv` and `csv → xlsx`
write 100 001 records because the header is one of them. For CSV, TSV, and
JSON, allocations do not grow with the row count. `csv → xlsx` on this input
is 55 because the data cells are plain numbers (the shared-string table is
the four header names) and the worksheet buffer plus the zip grow by doubling.
A column of distinct text allocates once per distinct string.
`cargo build --release --lib --target wasm32-unknown-unknown` produces
`libexport_sheet.rlib` for the CSV and JSON parsers.

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
is ignored. Delimited and JSON conversions are streaming: one record, plus a
fixed read window, not the table.

## XLSX

One worksheet, named Sheet. Text is a shared string. A plain number
(`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`) is stored as those
characters, not through a float. `3.50` stays `3.50`. A leading zero such as
`007` stays text. Empty cells are written, so a trailing blank field survives.
Column 27 is `AA`.

The writer stores every part (zip method 0). There is no compressor in this
crate. The reader accepts method 0 and method 8. Method 8 goes through a raw
DEFLATE inflater checked against known streams: empty, `hello`, `abc` repeated
50 times, a short worksheet snippet, and `café — 表`. Any other method, zip64,
encryption, a CRC or size that does not match, or a deflate stream that
inflater rejects is refused. The file is not guessed.

Only the first sheet is read. JSON is not an xlsx pair. A ragged delimited
file is still an error on the way into xlsx. XML that is not UTF-8 is refused.
CSV and JSON still repair invalid UTF-8 and count it.
