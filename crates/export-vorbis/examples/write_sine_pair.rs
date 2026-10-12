//! Write the two-frame file with the continuation second. The library does not call ffmpeg.

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sine-pair.ogg".to_string());
    let packet = export_vorbis::encode_half_scale_sine_pair().expect("encode");
    std::fs::write(&path, packet).expect("write");
}
