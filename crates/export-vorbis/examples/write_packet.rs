//! Write the one short mono packet to a path. The library does not call ffmpeg.

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "frame.ogg".to_string());
    let pcm = [0.0f32; export_vorbis::FRAME_SAMPLES];
    let packet = export_vorbis::encode_packet(&pcm).expect("encode");
    std::fs::write(&path, &packet).expect("write");
}
