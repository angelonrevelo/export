# export-translate

English ↔ Spanish for a small server. Not NLLB.

NLLB-200 distilled 600M, int8, stays resident at about 1.2 GB. This crate is a frequent-word lexicon plus morphology: a byte scanner, a binary search, and a clause walk. Zero dependencies. The library builds for `wasm32-unknown-unknown`. Static tables are a few hundred kilobytes, not model weights.

## Language-pair limit

Only **en → es** and **es → en** are translated. **en → en** and **es → es** return the input unchanged. Every other code — `tl`, `fil`, `zh`, `ja`, `fr`, `pt`, `de`, and the rest of Export's list — returns `unsupported language pair`. The source is not copied through and called a translation.

The lexicon is open: about 800 English lemmas (nouns, verbs, adjectives, adverbs, and function words) plus regular and irregular inflection. Closed entries still win on an exact hit, so the original fixture grammar is unchanged when a sentence stays inside it. A clause that contains any frequent or inflected token is walked left to right. That covers ordinary transcript, subtitle, OCR, and note sentences: past tense, `need to` / `can` / `will` plus an infinitive, `said` / `think` plus `que`, `talk to` → `con`, `was` / `were` as `estar` or `ser` from the adjective, and one object clitic before the verb.

An unknown token is copied and counted when at least one content word translates. Determiners, prepositions, conjunctions, pronouns, `be`, `not`, and `please` do not count. If no content word translates, the call returns an error and does not return the source.

`they` is `ellos`. Spanish `de` comes back as `of`. `agua` takes `el`. `nuestro` agrees; other possessives are `mi` / `tu` / `su`. On the way back, `con` is `with`, so `hablé con` is not guaranteed to return `talked to`. Subjunctive, relative clauses, clitic climbing, and any language that is not English or Spanish are out. No second language pair. No NLLB weights.

`PAIR_LIMIT` in the crate is the same statement the error path uses.

## Measure

Release, one paragraph that is not a stored fixture, both directions, 2,000 iterations (12,000 sentence translations). Three sentences:

> Yesterday I talked to my friend about the meeting. She said we need to finish the report before Friday, but the train was late and we missed it. I think we can send the notes tomorrow.

Spanish from that run:

> Ayer yo hablé con mi amigo sobre la reunión. Ella dijo que nosotros necesitamos terminar el informe antes del viernes, pero el tren estaba tarde y nosotros lo perdimos. Yo creo que nosotros podemos enviar las notas mañana.

| | |
|---|---|
| time per sentence | **2,273 ns** (2.273 µs) |
| peak heap during `translate` | **2,632 bytes** |
| process VmHWM | **2,691,072 bytes** (~2.6 MB) |
| lexicon static bytes | **211,012** |

Two more release runs of the same binary were 2,250 ns and 2,275 ns. Heap stayed 2,632 bytes. VmHWM stayed between 2,584,576 and 2,691,072 bytes. That resident set is what to compare with NLLB's ~1.2 GB.

The older fixture harness is still `export-translate-bench`. It times the closed sentences only. The numbers above are the ones for this limit.

```sh
cargo test --manifest-path crates/export-translate/Cargo.toml
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-paragraph
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-bench
cargo build --release --manifest-path crates/export-translate/Cargo.toml --target wasm32-unknown-unknown --lib
```

The bench binaries are native. They own the allocator counter and read `VmHWM` from `/proc/self/status`. The library does not. `build.rs` only compiles `data/freq.tsv` on the host; the library still has zero dependencies.
