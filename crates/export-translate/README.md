# export-translate

English ↔ Spanish and English ↔ Filipino for a small server. Not NLLB.

NLLB-200 distilled 600M, int8, stays resident at about 1.2 GB. This crate is a frequent-word lexicon plus morphology: a byte scanner, a binary search, and a clause walk. Zero dependencies. The library builds for `wasm32-unknown-unknown`. Static tables are a few hundred kilobytes, not model weights.

## Language-pair limit

**en ↔ es** and **en ↔ tl** are translated. **en → en**, **es → es**, and **tl → tl** return the input unchanged. `fil` is a different code from `tl` and stays unsupported, along with `zh`, `ja`, `fr`, `pt`, `de`, and every other code. Those calls return `unsupported language pair` and do not echo the source. The error text is unchanged and still says this crate translates en ↔ es only.

Spanish coverage is about 800 English lemmas plus regular and irregular inflection. Closed entries still win on an exact hit, so the original fixture grammar is unchanged when a sentence stays inside it. A clause that contains any frequent or inflected token is walked left to right: past tense, `need to` / `can` / `will` plus an infinitive, `said` / `think` plus `que`, `talk to` → `con`, `was` / `were` as `estar` or `ser` from the adjective, and one object clitic before the verb.

Filipino coverage is 465 English lemmas in `data/tl.tsv`, with `mag-` / `-um-` / reduplication and explicit object-focus forms (`tapusin` / `tinapos`, `ipadala`, `sabihin` / `sinabi`). `we` is inclusive (`tayo` / `natin`). Actor-focus verbs take an `ang` clitic; object-focus verbs take a genitive actor (`ng` or `ko` / `niya` / `natin`). `can` / `need` / `want` take a linker and an infinitive. A hyphen is glued only inside the Filipino walk.

An unknown token is copied and counted when at least one content word translates. Determiners, prepositions, conjunctions, pronouns, auxiliaries, and `be` do not count. If no content word translates, the call returns an error and does not return the source.

`they` is `ellos`. Spanish `de` comes back as `of`. `agua` takes `el`. `nuestro` agrees; other possessives are `mi` / `tu` / `su`. On the way back, `con` is `with`, so `hablé con` is not guaranteed to return `talked to`. Subjunctive, relative clauses, and clitic climbing are out. Filipino questions, relative clauses, and a second object are out. No NLLB weights.

`PAIR_LIMIT` in the crate is the coverage statement.

## Measure

Release, one Filipino paragraph that is not a stored fixture, both directions, 2,000 iterations (12,000 sentence translations). Three sentences, `cargo run --release --bin export-translate-tl`:

> Yesterday the team finished the report about the meeting. I will call my friend tomorrow and send the notes. She said the train was late, but we can meet at the office before Friday.

Filipino from that run:

> Kahapon tinapos ng koponan ang ulat tungkol sa pulong. Tatawag ako sa kaibigan ko bukas at ipadala ang mga tala. Sinabi niya na huli ang tren, pero maaari tayong magkita sa opisina bago ang Biyernes.

| | |
|---|---|
| time per sentence | **1,709 ns** (1.709 µs) |
| peak heap during `translate` | **1,862 bytes** |
| process VmHWM | **2,891,776 bytes** (~2.8 MB) |
| lexicon static bytes | **321,621** |

Two more release runs of the same binary were 1,643 ns and 1,630 ns. Heap stayed 1,862 bytes. VmHWM stayed between 2,846,720 and 2,891,776 bytes. That resident set is what to compare with NLLB's ~1.2 GB.

The English ↔ Spanish paragraph harness is still `export-translate-paragraph` and was not rewritten. Its paragraph still translates to:

> Ayer yo hablé con mi amigo sobre la reunión. Ella dijo que nosotros necesitamos terminar el informe antes del viernes, pero el tren estaba tarde y nosotros lo perdimos. Yo creo que nosotros podemos enviar las notas mañana.

A re-run on this build, after the Filipino tables were added to `lexicon_static_bytes`, was 2,038 ns per sentence, peak heap 2,632 bytes, VmHWM 2,891,776, static bytes 321,621. The earlier en-only-static figure was 211,012 bytes, at 2,273 ns (repeat runs 2,250 and 2,275) and VmHWM between 2,584,576 and 2,691,072. The fixture harness `export-translate-bench` is also unchanged.

```sh
cargo test --manifest-path crates/export-translate/Cargo.toml
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-paragraph
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-bench
cargo build --release --manifest-path crates/export-translate/Cargo.toml --target wasm32-unknown-unknown --lib
```

The bench binaries are native. They own the allocator counter and read `VmHWM` from `/proc/self/status`. The library does not. `build.rs` compiles `data/freq.tsv` and `data/tl.tsv` on the host; the library still has zero dependencies.
