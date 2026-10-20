# Playing music in coldsonic

Playing music needs three pieces:

1. **Decoding** — compressed audio (mp3, ogg, flac, aac, wav) → PCM.
2. **Output** — PCM to the sound card through the OS audio stack.
3. **A source of bytes** — today `Manager::track_bytes` (offline files; a
   networked provider later).

Nothing external like `mpv`/`gstreamer`/`vlc` is required. The only
system-level need is the OS audio stack that cpal links against (and ALSA
headers at **build** time on Linux).

## rodio 0.22

[`rodio`](https://crates.io/crates/rodio) 0.22 (Symphonia + cpal). Default
features decode flac/mp3/mp4(aac)/vorbis/wav.

The old 0.19/0.20 `OutputStream` / `Sink` API is gone. Current shape, as used
by `src/engine/player.rs`:

```rust
let mut sink = rodio::DeviceSinkBuilder::open_default_sink()?;
// MixerDeviceSink owns the cpal::Stream and is !Send — keep it on the UI thread.
let player = rodio::Player::connect_new(sink.mixer()); // Send + Sync
let source = rodio::Decoder::builder()
    .with_data(std::io::Cursor::new(bytes))
    .with_byte_len(bytes.len() as u64) // enables seeking + duration
    .build()?;
player.append(source);
player.play();
// pause / stop / skip_one / set_volume / try_seek / get_pos / empty
```

`Decoder::new(Cursor::new(bytes))` still works but does not mark the stream
seekable. We always set `with_byte_len` so `Player::try_seek` has a chance.

### Seek findings

`Player::try_seek` goes through rodio's wrapper chain
(`speed` / `track_position` / `pausable` / `amplify` / `skippable` /
`stoppable` / `periodic_access`). With `with_byte_len` it succeeds for the
formats we care about (in-memory Cursor is `Seek`). If it still returns
`SeekError::NotSupported`, the engine re-appends the same bytes and tries
once more; a second failure restarts from 0.

### Linux build-time ALSA headers

`cpal` needs ALSA dev headers **to build** (even on PipeWire/Pulse):

```
Debian/Ubuntu: sudo apt install libasound2-dev
Fedora:        sudo dnf install alsa-lib-devel
```

The Nix flake / devenv already pass `alsa-lib` + `pkg-config`. Runtime audio
still goes through PipeWire/PulseAudio/ALSA as usual.

## Where this lives

- `src/engine/player.rs` — `AudioPlayer` (sink + player).
- `src/engine/real.rs` — queue, fetch via `Manager::track_bytes`, auto-advance.
- Disk cache of those bytes is engine Facet 3, not here.
