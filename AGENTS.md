# Agent standing orders — export

Read before opening a pull request. These override any per-crate habit.

## The one target

The Rust crates exist to replace Python conversions **on real files**. Progress is
measured by one number, from `scripts/outside_score.py`:

> **N of M** — counted real files whose Rust output matches the Python app's
> output, or stays inside the tolerance the scorer documents.

A change counts only if it can raise N. Generated fixtures, one more size or item
on a ladder, and timing rows do not raise N and are not accepted on their own.

## What the scorer needs from Rust

N is 0 until a Rust command exists for the scorer to call. Build that first:

```
EXPORT_RUST_ORACLE=<path to binary>
<binary> <src> <dest> <kind>
```

`kind` is one of `phone-jpeg`, `heic`, `mp4`, `docx`, `pdf`, `xlsx`. Write exactly
one file at `dest`; exit non-zero on failure. What each kind must produce is the
Python reference in `python_reference()` in `scripts/outside_score.py`:

| kind | Python reference | match |
|---|---|---|
| `phone-jpeg`, `heic` | shrink to a 1,000,000-byte cap, padded to the cap | same length, JPEG, decodes to the same pixel size |
| `mp4` | shrink to a 5,000,000-byte cap | same length, MP4, same duration within 0.5 s |
| `docx` | → `txt` | exact bytes |
| `pdf` | → text | exact bytes |
| `xlsx` | → `csv` | exact bytes |

A source already under the cap is copied by the app; that is an exact match.

## The corpus is private

Real files live in a gitignored `corpus/` on the owner's machine. You cannot read
it. The owner runs the score and posts **counts and failure reasons, never file
names**, to the GitHub issue titled "Outside score". Read the latest comment there
before choosing work; aim at the largest failure bucket.

Do not commit real user files, and do not add generated files to make M larger.

## Gate — before any pull request

- `bash scripts/gate.sh` passes (clippy `-D warnings` on current stable, tests, wasm).
  It must pass on Windows too: no test may pin an `f32`/`f64` result to the last
  bit — use a tolerance.
- Example and binary names are unique across the workspace.
- `pytest tests/test_outside_score.py` passes.
- The pull request states the expected change to N, by kind.

## Do not

- Merge your own pull request. The owner merges.
- Remove or rewrite deploy or security documentation (`README.md` "Deploying",
  `SECURITY.md`).
- Add another SHA-256, CRC-32 or DEFLATE implementation; use `export-kernel`.
