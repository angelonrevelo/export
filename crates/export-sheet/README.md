# export-sheet

Streaming conversions for a rectangle of cells: CSV, TSV, one JSON array, one
BIFF8 `.xls` workbook, and one XLSX worksheet. No dependencies. The parsers are
in this crate.

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

A second generated file keeps B1 as `cat` and adds a `NUMBER` in C1. The three
values are **42**, **cat**, and **7**. `xls → csv` prints `42,cat,7`.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 603 ns |
| p99 | 780 ns |

A third generated file is one sheet, 1 000 rows, one numeric column, values 0
through 999. There is no formula record. The file is **19 968 bytes**.
`xls → csv` writes 1 000 lines.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 138 342 ns |
| p99 | 213 695 ns |

The same sheet at the next count is 2 000 rows, values 0 through 1 999. One
numeric column, no formulas. Every row reads back. The file is **37 888 bytes**.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 263 461 ns |
| p99 | 354 234 ns |

The same sheet at the next count is 4 000 rows, values 0 through 3 999. One
numeric column, no formulas. Every row reads back. The file is **74 240 bytes**.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 532 990 ns |
| p99 | 676 645 ns |

A fourth generated file has A1 = 2, B1 = 3, and C1 = a SUM formula. The reader
returns the cached number **5** and does not evaluate the expression.
`xls → csv` prints `2,3,5`.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 575 ns |

When that SUM has no cached number, the reader adds the two numeric cells and
does not parse any other expression. A1 = 8 and B1 = 9, so the value is **17**.
`xls → csv` prints `8,9,17`.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 669 ns |

A fifth generated file has A1 = 4, B1 = 5, C1 = 6, and D1 = a SUM of those
three cells with no cached number. The reader adds them. The value is **15**.
`xls → csv` prints `4,5,6,15`. Any other uncached expression is still refused.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 767 ns |

A sixth generated file extends that SUM by one numeric cell, still with no
cached number. A1 = 4, B1 = 5, C1 = 6, and D1 = 7. The reader adds them. The
value is **22**. `xls → csv` prints `4,5,6,7,22`. Any other uncached
expression is still refused.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 921 ns |

A seventh generated file is a SUM of one contiguous three-cell range, still
with no cached number. The formula is one area token. A1 = 1, B1 = 2, and
C1 = 3, so the value is **6**. `xls → csv` prints `1,2,3,6`. Any other
uncached expression is still refused.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 763 ns |

An eighth generated file widens that area by one cell, still with no cached
number. A1 = 1, B1 = 2, C1 = 3, and D1 = 4. The reader adds them. The value
is **10**. `xls → csv` prints `1,2,3,4,10`. Any other uncached expression is
still refused.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 881 ns |

A ninth generated file turns that area down column A. A1 = 1, A2 = 2, and
A3 = 3. The formula is the same area token. The value is **6**. `xls → csv`
prints `1`, `2`, `3`, and then `6`. Any other uncached expression is still
refused.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 807 ns |

A tenth generated file lengthens that column by one cell, still with no
cached number. A1 = 1, A2 = 2, A3 = 3, and A4 = 4. The formula is the same
area token. The value is **10**. `xls → csv` prints `1`, `2`, `3`, `4`, and
then `10`. Any other uncached expression is still refused.

Release build, same method (10 000 reads, 200 warmup reads, nearest rank):

| | time |
|---|---:|
| p50 | 966 ns |

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
