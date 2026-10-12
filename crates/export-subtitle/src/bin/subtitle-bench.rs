use export_subtitle::{cues_to_srt, parse_srt};
use std::time::Instant;

fn main() {
    let mut input = String::new();
    for i in 0..10_000 {
        let start = i % 60;
        let end = (start + 1) % 60;
        input.push_str(&format!(
            "{n}\n00:00:{start:02},000 --> 00:00:{end:02},500\nCue {n}\n\n",
            n = i + 1
        ));
    }
    if cfg!(debug_assertions) {
        println!("cue parse: NOT JUDGED (debug build)");
        return;
    }
    let start = Instant::now();
    let cues = parse_srt(&input).expect("cues");
    let _ = cues_to_srt(&cues);
    let secs = start.elapsed().as_secs_f64();
    let rate = cues.len() as f64 / secs;
    println!("srt parse+write {:.0} cues/s ({} cues)", rate, cues.len());
}
