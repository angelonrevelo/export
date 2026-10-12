# export-vorbis

One Vorbis audio packet: mono, 48 kHz, one short silent frame (32 PCM samples from a 64-sample block).

`encode_packet` writes that packet. `decode_packet` reads it back to PCM. A truncated packet or a bad header returns an error. Stereo returns unsupported.

This is not a song encoder. Threading is off. There are no dependencies and no rustls.

ffmpeg treats the first Vorbis frame as delay and drops it. Decode this packet with:

```
ffmpeg -flags2 +skip_manual -i frame.ogg -f f32le pcm.raw
```

`encode_half_scale_sine` writes two short frames of one half-scale 440 Hz sine, same rate and length, plus the overlap delay those frames need. Default ffmpeg drops the delay. The second frame is 32 samples, peak 0.50578, error RMS 0.00510 against that sine.

A later frame is samples n = 32 through 63 of that sine, after the overlap frame it needs. ffmpeg returns 32 samples, peak 0.47849, error RMS 0.00442 against `0.5 * sin(2π * 440 * n / 48000)` for those n.

`encode_half_scale_sine_pair` puts that continuation in the second position of a two-frame file. ffmpeg returns 64 samples. The second frame is 32 samples, peak 0.50916, error RMS 0.00597 against that sine for n = 32 through 63.

A third frame in that file is n = 64 through 95. ffmpeg returns 96 samples. That frame is 32 samples, peak 0.49088, error RMS 0.61950. That error RMS is above 0.02, so the file does not continue.

The same audio packet with no frame before it does not match the sine. Default ffmpeg emits 0 samples. With `-flags2 +skip_manual` it emits 32 samples, peak 0.50377, error RMS 0.25002.
