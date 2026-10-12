# Limits

This is not a video decoder. It replaces one FFmpeg VP9 keyframe hop for a single generated picture.

Supported input:

- IVF (`DKIF` / `VP90`) containing exactly one frame
- profile 0, 8-bit, 4:2:0
- 8×8, 16×16, 32×32, 64×64, 128×128, 256×256, or 512×512 keyframe, shown, no separate render size
- lossless quantizer, so only 4×4 Walsh-Hadamard transforms
- DC luma and DC chroma
- segmentation off, loop filter level 0, one tile
- no compressed-header probability updates

Anything else returns `Error::Unsupported`. A short buffer returns `Error::Truncated`. A wrong marker, sync code, fourcc, or bool-coder padding returns `Error::BadHeader`.

The crate does not start threads. Release builds use `opt-level = 3`, thin LTO, and one codegen unit. The library target `wasm32-unknown-unknown` builds as a normal rlib; this crate does not ship a JavaScript wrapper.
