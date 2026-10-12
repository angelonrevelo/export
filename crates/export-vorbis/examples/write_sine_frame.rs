//! Write the sine frame with no audio frame before it. The library does not call ffmpeg.

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sine-frame.ogg".to_string());
    let packet = export_vorbis::encode_half_scale_sine_frame().expect("encode");
    std::fs::write(&path, packet).expect("write");
}
