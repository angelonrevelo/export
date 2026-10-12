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

A second paragraph, also not a stored fixture, uses only lemmas and clause shapes already in the crate. Three sentences, both directions, 2,000 iterations. Each sample is that iteration's time divided by sentences × 2. Nearest-rank percentiles, `cargo run --release --bin export-translate-tl-pctl`:

> Today the teacher opened the new book about the city. The children want to play outside, but the weather is cold. I think we should buy food at the market before Sunday.

> Ngayon binuksan ng guro ang bagong aklat tungkol sa lungsod. Gustong maglaro ang mga bata sa labas, pero malamig ang panahon. Sa palagay ko na dapat tayong bumili ng pagkain sa palengke bago ang Linggo.

| | |
|---|---|
| sentences | **3** |
| p50 per sentence | **1,510 ns** (1.510 µs) |
| p99 per sentence | **1,970 ns** (1.970 µs) |

A second release run of that binary was p50 1,508 ns and p99 2,255 ns. Both medians are within 1.88 µs.

A six-sentence paragraph, also not a stored fixture, uses only lemmas and clause shapes already in the crate. Both directions, 2,000 iterations. Each sample is that iteration's time divided by sentences × 2. Nearest-rank percentiles, `cargo run --release --bin export-translate-tl-six`:

> The mother cooked rice today. The children drank milk at home. She opened the letter from the teacher. We can walk to school tomorrow. The dog ran outside, but the cat is sleeping at home. I think the weather is good.

> Nagluto ang ina ng kanin ngayon. Uminom ang mga bata ng gatas sa tahanan. Binuksan niya ang liham mula sa guro. Maaari tayong maglakad sa paaralan bukas. Tumakbo ang aso sa labas, pero natutulog ang pusa sa tahanan. Sa palagay ko na mabuti ang panahon.

| | |
|---|---|
| sentences | **6** |
| p50 per sentence | **1,051 ns** (1.051 µs) |
| p99 per sentence | **1,334 ns** (1.334 µs) |

A second release run of that binary was p50 1,047 ns and p99 1,334 ns. Both medians are within 1.66 µs.

A twelve-sentence paragraph, also not a stored fixture, uses only lemmas and clause shapes already in the crate. Both directions, 2,000 iterations. Each sample is that iteration's time divided by sentences × 2. Nearest-rank percentiles, `cargo run --release --bin export-translate-tl-twelve`:

> The father bought bread yesterday. She wrote a story at school. The boy drank water at home. We can read the book tomorrow. The girl played outside, but the cat is sleeping inside. I think the house is new. He opened the door today. They want to eat rice. The teacher came from the city. We should study the book tomorrow. My friend saw the bird yesterday. The children are playing at school.

> Bumili ang ama ng tinapay kahapon. Sumulat siya ng isang kuwento sa paaralan. Uminom ang lalaki ng tubig sa tahanan. Maaari nating basahin ang aklat bukas. Naglaro ang babae sa labas, pero natutulog ang pusa sa loob. Sa palagay ko na bago ang bahay. Binuksan niya ang pinto ngayon. Gusto silang kumain ng kanin. Dumating ang guro mula sa lungsod. Dapat tayong mag-aral ng aklat bukas. Nakita ng kaibigan ko ang ibon kahapon. Naglalaro ang mga bata sa paaralan.

| | |
|---|---|
| sentences | **12** |
| p50 per sentence | **923 ns** (0.923 µs) |
| p99 per sentence | **1,253 ns** (1.253 µs) |

A second release run of that binary was p50 924 ns and p99 1,210 ns. Both medians are within 1.16 µs.

A twenty-four-sentence paragraph, also not a stored fixture, uses only lemmas and clause shapes already in the crate. Both directions, 2,000 iterations. Each sample is that iteration's time divided by sentences × 2. Nearest-rank percentiles, `cargo run --release --bin export-translate-tl-twentyfour`:

> The doctor drove the car today. She sold bread at the market. The boy found the key at home. We can walk to the office tomorrow. The woman cooked fish, but the man is sleeping. I think the coffee is hot. He closed the window yesterday. They want to drink tea. The student came from school. We should buy food tomorrow. My mother saw the cat today. The children are working at home. The father called the doctor yesterday. She sent the letter today. The girl drank milk at school. We can eat rice tomorrow. The dog ran to the street, but the bird is sleeping. I think the tea is good. He read the note today. They need to help the family. The friend lived in the city. My father found money yesterday. She closed the door today. The children are reading at school.

> Nagmaneho ang doktor ng kotse ngayon. Nagbenta siya ng tinapay sa palengke. Hinanap ng lalaki ang susi sa tahanan. Maaari tayong maglakad sa opisina bukas. Nagluto ang babae ng isda, pero natutulog ang lalaki. Sa palagay ko na mainit ang kape. Isinara niya ang bintana kahapon. Gusto silang uminom ng tsaa. Dumating ang estudyante mula sa paaralan. Dapat tayong bumili ng pagkain bukas. Nakita ng ina ko ang pusa ngayon. Nagtratrabaho ang mga bata sa tahanan. Tumawag ang ama sa doktor kahapon. Ipinadala niya ang liham ngayon. Uminom ang babae ng gatas sa paaralan. Maaari tayong kumain ng kanin bukas. Tumakbo ang aso sa kalye, pero natutulog ang ibon. Sa palagay ko na mabuti ang tsaa. Binabasa niya ang tala ngayon. Kailangan silang tumulong ng pamilya. Tumira ang kaibigan sa lungsod. Hinanap ng ama ko ang pera kahapon. Isinara niya ang pinto ngayon. Binabasa ng mga bata sa paaralan.

| | |
|---|---|
| sentences | **24** |
| p50 per sentence | **911 ns** (0.911 µs) |
| p99 per sentence | **1,152 ns** (1.152 µs) |

A second release run of that binary was p50 905 ns and p99 1,181 ns. Both medians are within 1.015 µs.

The English ↔ Spanish paragraph harness is still `export-translate-paragraph` and was not rewritten. Its paragraph still translates to:

> Ayer yo hablé con mi amigo sobre la reunión. Ella dijo que nosotros necesitamos terminar el informe antes del viernes, pero el tren estaba tarde y nosotros lo perdimos. Yo creo que nosotros podemos enviar las notas mañana.

A re-run on this build, after the Filipino tables were added to `lexicon_static_bytes`, was 2,038 ns per sentence, peak heap 2,632 bytes, VmHWM 2,891,776, static bytes 321,621. The earlier en-only-static figure was 211,012 bytes, at 2,273 ns (repeat runs 2,250 and 2,275) and VmHWM between 2,584,576 and 2,691,072. The fixture harness `export-translate-bench` is also unchanged.

```sh
cargo test --manifest-path crates/export-translate/Cargo.toml
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl-pctl
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl-six
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl-twelve
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl-twentyfour
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-paragraph
cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-bench
cargo build --release --manifest-path crates/export-translate/Cargo.toml --target wasm32-unknown-unknown --lib
```

The bench binaries are native. They own the allocator counter and read `VmHWM` from `/proc/self/status`. The library does not. `build.rs` compiles `data/freq.tsv` and `data/tl.tsv` on the host; the library still has zero dependencies.
