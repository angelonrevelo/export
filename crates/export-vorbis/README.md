# export-vorbis

One Vorbis audio packet: mono, 48 kHz, one short silent frame (32 PCM samples from a 64-sample block).

`encode_packet` writes that packet. `decode_packet` reads it back to PCM. A truncated packet or a bad header returns an error. Stereo returns unsupported.

This is not a song encoder. Threading is off. There are no dependencies and no rustls.

ffmpeg treats the first Vorbis frame as delay and drops it. Decode this packet with:

```
ffmpeg -flags2 +skip_manual -i frame.ogg -f f32le pcm.raw
```

`encode_half_scale_sine` writes one half-scale 440 Hz frame, same rate and length, plus the overlap delay that frame needs. Default ffmpeg drops the delay and returns those 32 samples:

```
ffmpeg -i sine.ogg -f f32le pcm.raw
```
