use export_subtitle::{cues_to_ass, cues_to_srt, parse_ass, parse_srt};
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
    } else {
        let start = Instant::now();
        let cues = parse_srt(&input).expect("cues");
        let _ = cues_to_srt(&cues);
        let secs = start.elapsed().as_secs_f64();
        let rate = cues.len() as f64 / secs;
        println!("srt parse+write {:.0} cues/s ({} cues)", rate, cues.len());
    }
    measure_one_bold();
    measure_one_italic();
    measure_one_underline();
}

/// Release latency for one bold dialogue line and one plain cue.
fn measure_one_bold() {
    let input = "\
[Script Info]\n\
ScriptType: v4.00+\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Hello {\\b1}there{\\b0} {\\i1}pal{\\i0}\n\
Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,Plain cue\n\
";
    let cues = parse_ass(input).expect("ass");
    assert_eq!(cues[0].text, "Hello {\\b1}there{\\b0} pal");
    assert_eq!(cues[1].text, "Plain cue");
    let written = cues_to_ass(&cues);
    assert!(written.contains("{\\b1}there{\\b0}"));
    let again = parse_ass(&written).expect("round trip");
    assert_eq!(again[0].text, cues[0].text);
    assert_eq!(again[1].text, "Plain cue");
    if cfg!(debug_assertions) {
        println!("ass bold: NOT JUDGED (debug build)");
        return;
    }
    const WARMUP: usize = 50;
    const SAMPLES: usize = 500;
    let mut sink = 0usize;
    for _ in 0..WARMUP {
        sink = sink.wrapping_add(pass(input));
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        sink = sink.wrapping_add(pass(input));
        samples.push(start.elapsed().as_nanos() as f64);
    }
    std::hint::black_box(sink);
    let p50 = percentile(&mut samples, 50.0);
    let p99 = percentile(&mut samples, 99.0);
    println!(
        "ass bold bytes in {} bytes out {} p50 {:.0} ns p99 {:.0} ns",
        input.len(),
        written.len(),
        p50,
        p99
    );
}

/// Release latency for one italic dialogue line and one plain cue.
fn measure_one_italic() {
    let input = "\
[Script Info]\n\
ScriptType: v4.00+\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Hello {\\i1}there{\\i0} {\\u1}pal{\\u0}\n\
Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,Plain cue\n\
";
    let cues = parse_ass(input).expect("ass");
    assert_eq!(cues[0].text, "Hello {\\i1}there{\\i0} pal");
    assert_eq!(cues[1].text, "Plain cue");
    let written = cues_to_ass(&cues);
    assert!(written.contains("{\\i1}there{\\i0}"));
    let again = parse_ass(&written).expect("round trip");
    assert_eq!(again[0].text, cues[0].text);
    assert_eq!(again[1].text, "Plain cue");
    if cfg!(debug_assertions) {
        println!("ass italic: NOT JUDGED (debug build)");
        return;
    }
    const WARMUP: usize = 50;
    const SAMPLES: usize = 500;
    let mut sink = 0usize;
    for _ in 0..WARMUP {
        sink = sink.wrapping_add(pass(input));
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        sink = sink.wrapping_add(pass(input));
        samples.push(start.elapsed().as_nanos() as f64);
    }
    std::hint::black_box(sink);
    let p50 = percentile(&mut samples, 50.0);
    let p99 = percentile(&mut samples, 99.0);
    println!(
        "ass italic bytes in {} bytes out {} p50 {:.0} ns p99 {:.0} ns",
        input.len(),
        written.len(),
        p50,
        p99
    );
}

/// Release latency for one underline dialogue line and one plain cue.
fn measure_one_underline() {
    let input = "\
[Script Info]\n\
ScriptType: v4.00+\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Hello {\\u1}there{\\u0} {\\s1}pal{\\s0}\n\
Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,Plain cue\n\
";
    let cues = parse_ass(input).expect("ass");
    assert_eq!(cues[0].text, "Hello {\\u1}there{\\u0} pal");
    assert_eq!(cues[1].text, "Plain cue");
    let written = cues_to_ass(&cues);
    assert!(written.contains("{\\u1}there{\\u0}"));
    let again = parse_ass(&written).expect("round trip");
    assert_eq!(again[0].text, cues[0].text);
    assert_eq!(again[1].text, "Plain cue");
    if cfg!(debug_assertions) {
        println!("ass underline: NOT JUDGED (debug build)");
        return;
    }
    const WARMUP: usize = 50;
    const SAMPLES: usize = 500;
    let mut sink = 0usize;
    for _ in 0..WARMUP {
        sink = sink.wrapping_add(pass(input));
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        sink = sink.wrapping_add(pass(input));
        samples.push(start.elapsed().as_nanos() as f64);
    }
    std::hint::black_box(sink);
    let p50 = percentile(&mut samples, 50.0);
    let p99 = percentile(&mut samples, 99.0);
    println!(
        "ass underline bytes in {} bytes out {} p50 {:.0} ns p99 {:.0} ns",
        input.len(),
        written.len(),
        p50,
        p99
    );
}

fn pass(input: &str) -> usize {
    let cues = parse_ass(input).expect("ass");
    cues_to_ass(&cues).len()
}

fn percentile(samples: &mut [f64], pct: f64) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    let n = samples.len();
    let rank = ((pct / 100.0) * n as f64).ceil() as usize;
    samples[rank.saturating_sub(1).min(n - 1)]
}
