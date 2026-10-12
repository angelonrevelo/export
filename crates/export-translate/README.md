# export-translate

English ↔ Spanish for a small server. Not NLLB.

NLLB-200 distilled 600M, int8, stays resident at about 1.2 GB. This crate is a closed lexicon and a shallow transfer: a byte scanner, a binary search, and a clause renderer. Zero dependencies. The library builds for `wasm32-unknown-unknown`.

## Language-pair limit

Only **en → es** and **es → en** are translated. **en → en** and **es → es** return the input unchanged. Every other code — `tl`, `fil`, `zh`, `ja`, `fr`, `pt`, `de`, and the rest of Export's list — returns `unsupported language pair`. The source is not copied through and called a translation.

The grammar is one subject noun phrase, one object, a copula (`ser` / `estar` from the adjective), `estar` + gerund, a tú-imperative for a bare base verb, and trailing adjuncts. `and` / `or` / `but` split clauses; the verb is not copied onto the next conjunct. `not` is only the copula pattern (`is not new` → `no es nueva`). `they` is `ellos`. Spanish `de` comes back as `of`. `agua` takes `el`. Unknown tokens are copied only when something else in the sentence hit the lexicon; if nothing hit, the call errors.

Outside this shape: past, subjunctive, clitics, reflexives, relative clauses, do-support, and any language that is not English or Spanish. The lexicon is 53 English surface forms and four verb paradigms (`open`, `save`, `play`, `want`). Phrases are at most three words (`how are you`, `thank you`, `good morning`). No fixture sentence is stored as a single entry.

`PAIR_LIMIT` in the crate is the same statement the error path uses.

## Measure

Release, 17 fixture sentences, both directions, 2,000 iterations (68,000 translations):

| | |
|---|---|
| time per sentence | **702 ns** (0.702 µs) |
| peak heap during `translate` | **1,447 bytes** |
| process VmHWM | **2,179,072 bytes** (~2.1 MB) |
| lexicon static bytes | **10,943** (string bytes counted once per reference, before the linker merges them) |

Two earlier release runs on the same binary shape were 682 ns and 737 ns. Heap stayed 1,447 bytes. VmHWM stayed between 2,174,976 and 2,244,608 bytes. That is the resident set to compare with NLLB's ~1.2 GB, not a second copy of the model.

```sh
cargo test --manifest-path crates/export-translate/Cargo.toml
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-bench
cargo build --release --manifest-path crates/export-translate/Cargo.toml --target wasm32-unknown-unknown --lib
```

The bench binary is native. It owns the allocator counter and reads `VmHWM` from `/proc/self/status`. The library does not.
