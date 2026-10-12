//! Release timings for the one short packet, including the truncated-packet error.

fn main() {
    let pcm = [0.0f32; export_vorbis::FRAME_SAMPLES];
    let packet = export_vorbis::encode_packet(&pcm).expect("encode");
    let truncated = &packet[..packet.len() / 2];

    for _ in 0..200 {
        let _ = export_vorbis::encode_packet(&pcm);
        let _ = export_vorbis::decode_packet(&packet);
        let _ = export_vorbis::decode_packet(truncated);
    }

    let rounds = 4000usize;
    let mut encode = Vec::with_capacity(rounds);
    let mut decode = Vec::with_capacity(rounds);
    let mut truncated_err = Vec::with_capacity(rounds);
    let mut combined = Vec::with_capacity(rounds * 3);

    for _ in 0..rounds {
        let start = std::time::Instant::now();
        export_vorbis::encode_packet(&pcm).expect("encode");
        push_ns(&mut encode, &mut combined, start);

        let start = std::time::Instant::now();
        export_vorbis::decode_packet(&packet).expect("decode");
        push_ns(&mut decode, &mut combined, start);

        let start = std::time::Instant::now();
        let err = export_vorbis::decode_packet(truncated).expect_err("truncated");
        assert_eq!(err, export_vorbis::Error::Truncated);
        push_ns(&mut truncated_err, &mut combined, start);
    }

    encode.sort_unstable();
    decode.sort_unstable();
    truncated_err.sort_unstable();
    combined.sort_unstable();

    println!("rounds={rounds}");
    print_pair("encode_ns", &encode);
    print_pair("decode_ns", &decode);
    print_pair("truncated_error_ns", &truncated_err);
    print_pair("combined_ns", &combined);
}

fn push_ns(series: &mut Vec<u128>, combined: &mut Vec<u128>, start: std::time::Instant) {
    let elapsed = start.elapsed().as_nanos();
    series.push(elapsed);
    combined.push(elapsed);
}

fn print_pair(name: &str, sorted: &[u128]) {
    println!(
        "{name} p50={} p99={}",
        percentile(sorted, 50),
        percentile(sorted, 99)
    );
}

fn percentile(sorted: &[u128], percent: usize) -> u128 {
    let rank = (percent * sorted.len() + 99) / 100;
    sorted[rank.saturating_sub(1)]
}
