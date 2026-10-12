//! Write two half-scale 440 Hz frames. The library does not call ffmpeg.

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sine.ogg".to_string());
    let packet = export_vorbis::encode_half_scale_sine().expect("encode");
    std::fs::write(&path, packet).expect("write");
}
