# Playing music in coldsonic

## What's needed

Playing music needs three pieces:

1. **Decoding** — turning compressed audio (mp3, ogg, flac, aac, wav) into raw PCM samples.
2. **Output/device playback** — sending PCM to the sound card through the OS audio stack.
3. **A streaming source** — for a Subsonic client this is the server's `stream` endpoint.

All of this is available as pure-Rust crates; nothing external like `mpv`/`gstreamer`/`vlc` is
required. The only system-level need is the standard OS audio stack that cpal/miniaudio link
against (and one small build-time dev package, see below).

## Recommended: rodio

[`rodio`](https://crates.io/crates/rodio) (0.22) is the standard, mature playback crate.

- **Decoding:** [Symphonia](https://crates.io/crates/symphonia) (pure Rust). Default features
  enable `flac`, `mp3`, `mp4`/`aac`, `ogg`/`vorbis`, `wav` — exactly what Subsonic servers serve.
- **Playback:** [`cpal`](https://crates.io/crates/cpal) (crate, not a system library).
- **Threading:** it drives the audio device on its own thread, so it does **not** block or fight
  the iced/winit event loop or the smol executor. Create the `Sink` once and `append()` from anywhere.

```toml
[dependencies]
rodio = "0.22"   # defaults already include playback + flac/mp3/mp4/vorbis/wav
```

### The one catch: Linux build-time ALSA headers

`cpal` needs ALSA dev headers **to build** on Linux (it's always needed, even on PipeWire/Pulse):

```
Debian/Ubuntu: sudo apt install libasound2-dev
Fedora:        sudo dnf install alsa-lib-devel
```

At runtime Linux audio still goes through PipeWire/PulseAudio/ALSA as usual — that's the OS audio
stack, not an app dependency. `rodio`/`cpal` picks the best available host at runtime.

## No-system-dependency alternative: miniaudio via maudio

If you want zero build-time dev packages, [`maudio`](https://crates.io/crates/maudio) (bindings to
[miniaudio](https://github.com/mackron/miniaudio)) vendors the C source, compiles it statically
(ships pre-generated bindings, needs only a C compiler), and `dlopen`s ALSA/PulseAudio at runtime.
Cons:

- `maudio` is young (0.1.x) and lower-level than rodio.
- Needs a C compiler in the build environment.
- You'd decode with Symphonia yourself and hand PCM to miniaudio.

`rminiaudio` also exists but needs `libasound2-dev` + `libclang-dev` to build, so it's not better
than rodio on that front.

## Getting the audio from Subsonic

`opensubsonic` already provides the stream source. `opensubsonic::Client::stream_url(id)` returns
a signed URL to the server's `/rest/stream` endpoint for a song id (media_retrieval.rs).

To play from a URL with rodio, `Decoder` expects a `Read + Seek` source, so **download/cache the
stream to a buffer first**, then feed the bytes to the decoder:

- fetch bytes with `reqwest` (or `client.get_bytes(id)`),
- `rodio::Decoder::new(Cursor::new(bytes))` → `Sink::append(source)`.

Alternatively use Symphonia's async/streaming decode directly (more work).

## Plan of attack (when implementing)

1. Add `rodio = "0.22"` to `Cargo.toml`. Install `libasound2-dev` on this machine.
2. Hold a `rodio::OutputStream` + `rodio::Sink` somewhere long-lived (an `AudioPlayer` shared
   between screens — likely passed into the player screen, not created per-song).
3. On play: `client.stream_url(id)` → fetch bytes → `Decoder` → `Sink::append`.

## Files that will touch this

- `Cargo.toml` — add `rodio`.
- New `src/audio.rs` module (player state, `OutputStream`/`Sink`).
- `src/screen/` — a player screen holding the audio state.