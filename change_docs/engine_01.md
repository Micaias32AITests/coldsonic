# Player engine — mock-first wiring, playback, disk cache with LRU

Applies to: `src/engine/*` (new), `src/screen/mod.rs`, `src/screen/artists.rs`,
`src/config.rs`, `Cargo.toml`, `sound.md`.

## Goal

Add a player **engine** to coldsonic: it streams audio from the server, caches
what it fetches, and (later) scrobbles and downloads. This doc covers the first
four facets:

1. **Interface + mock engine + wiring** — define the engine's public types, build
   a mock engine behind that seam, and wire the whole UI flow against it
   (artists play-shuffled → queue → now-playing bar) to validate the
   event/coordinator model before any audio exists.
2. **Real playback engine** — rodio 0.22 behind the same seam: queue,
   auto-advance, direct streaming fetch (no cache yet).
3. **Disk cache with LRU + config** — bounded, least-recently-used eviction,
   cache-aware fetching, size capped via `settings.toml`.
4. **Extras (later)** — prefetch, scrobbling, downloads, cover-cache migration.

Facets 1–3 are separate, verifiable milestones; each builds on the previous.

## Locked in decisions

- **Event model:** the engine never talks to screens directly. It emits
  `EngineEvent`s upward to its owner — `MainScreen` is the coordinator — which
  pushes state down to the views that need it (now-playing bar today, a player
  screen later). Screens raise *intents* (`PlayQueue`, `Pause`, …) and read
  pushed state; they never touch audio.
- **Mock-first:** Facet 1 ships zero new audio/network dependencies and validates
  the pipe with a fake engine.
- **Track type:** `pub type Track = opensubsonic::data::Child;` — already carries
  `id` (fetching), `duration`, `title`/`artist`/`album`, `cover_art`. Filters:
  only playable entries (`!is_dir`), usually via album song lists.
- **Engine owner:** `MainScreen` (the coordinator). Lift to top-level `State`
  only if a need outside it appears.
- **Tick:** 250 ms, `const` — fast enough to notice track-end within ~1/4 s and
  keep the bar smooth; rodio tracks position internally at 5 ms so we only read
  a value. Tunable in one line.
- **No in-memory cache** for streams (they're tens of MB). RAM holds only the
  current track's bytes. Disk is the cache.
- **Cache bound is a config knob** from day one (`settings.toml`), not a constant.
- **rodio 0.22 note:** `sound.md` documents the old 0.19/0.20 API
  (`OutputStream`/`Sink`). Current API: `DeviceSinkBuilder::open_default_sink()`
  → `MixerDeviceSink` (owns the `cpal::Stream`, `!Send`) + `Player::connect_new(&mixer)`
  (`Send + Sync`, auto-tracks position, `append(source)`, `play/pause/stop/clear/
  skip_one/try_seek/set_volume/get_pos/empty`). `Decoder::new(Cursor::new(bytes))`
  plays in-memory bytes; default features decode flac/mp3/mp4(aac)/vorbis/wav.
  Update `sound.md` in Facet 2.

## Facet 1 — Interface, mock engine, wiring (no new audio deps)

### Engine interface (the seam)

`src/engine/mod.rs`:

```rust
/// What we play. See "Locked in decisions" — Child carries everything we need.
pub type Track = opensubsonic::data::Child;

pub enum EngineMsg {
    PlayQueue(Vec<Track>),   // replace queue, play first
    PlayNext(Track),         // play now, insert at front
    Enqueue(Track),          // append to queue end
    Next,                    // skip to next queued track
    Pause, Resume, Toggle, Stop,
    SetVolume(f32),
    Seek(std::time::Duration), // Facet 2+ (best-effort)
    /// Internal: async fetch finished (Facet 2+).
    BytesReady { generation: u64, result: Result<bytes::Bytes, EngineError> },
}

pub enum EngineEvent {
    PlaybackStarted(Track),
    TrackFinished(Track),          // scrobble-submission hook (Facet 4)
    Paused, Resumed, Stopped, QueueChanged,
    Error(String),
}

pub struct NowPlaying {            // snapshot pushed to UI
    pub track: Track,
    pub position: std::time::Duration,
    pub duration: std::time::Duration,
    pub state: PlaybackState,      // Stopped | Loading | Playing | Paused
}
```

`Engine` is an enum wrapper so `MainScreen` keeps `#[derive(Debug, Clone)]`:

```rust
pub enum Engine {
    Mock(MockEngine),
    Real(RealEngine),              // Facet 2+
}

impl Engine {
    pub fn update(&mut self, msg: EngineMsg) -> Action<EngineMsg, EngineEvent>;
    /// Called by MainScreen on each tick. Returns events to push to the UI.
    pub fn tick(&mut self) -> Vec<EngineEvent>;
    pub fn now_playing(&self) -> Option<NowPlaying>;
}
```

`EngineError` = `thiserror` enum wrapping `opensubsonic::Error` (+ decode errors
later). Reuses the crate's `Action<T, U>` (`None | Task | Emit`) so engine
commands machine with the same plumbing as screens.

If `MainScreen`/`State` cannot keep deriving `Clone` once the real engine exists
(rodio handles are not `Clone`), drop the `Clone` derives on those two types —
iced does not require them.

### Mock engine behavior

`src/engine/mock.rs` — same seam, no audio:

- Every incoming `EngineMsg` is logged (`eprintln!`, matching current style).
- Keeps a fake `VecDeque<Track>` + fake position. On `PlayQueue`: store queue,
  set `current`, emit `QueueChanged` + `PlaybackStarted`. On `tick`: advance
  position (simulate ~1 s per tick so the bar visibly moves), and after a short
  fake duration (10 s, `const`) emit `TrackFinished`, advance to next queued
  track (auto-advance), repeat. `Pause/Resume/Toggle/Stop/Next/PlayNext/Enqueue`
  behave and emit.
- `now_playing()` returns the current track + fake position/duration.

### Wiring (this facet only)

- **`src/screen/mod.rs` (MainScreen):**
  - Fields: `engine: Engine`, `now_playing: Option<engine::NowPlaying>`
    (replaces `SongPlaying`; bar keeps cover display but mock has no bytes →
    placeholder space in Facet 1).
  - New msgs: `Msg::Engine(engine::EngineMsg)`, `Msg::Tick`.
  - `Screen::subscription`: `Subscription::run` with a static builder —
    `iced::stream::channel` looping `smol::Timer::after(250ms)` → `Msg::Tick`.
    No engine data crosses into the subscription; `update` reads the engine.
  - `update`: route `Msg::Engine` through `engine.update(...).handle(...)`;
    on `Msg::Tick` call `engine.tick()` and apply events to `now_playing`
    (also log them — this is the "watch the events flow" verification).
  - Now-playing bar: shows `now_playing` (title/artist, position/duration from
    the snapshot) + minimal control buttons **pause/resume/next** emitting
    `Msg::Engine(...)` — exercising the full pipe.
- **`src/screen/artists.rs`:**
  - `type Emit = Never` → `pub enum Event { PlayShuffled(String /* artist id */) }`.
  - `Msg::PlayShuffled` handler: `Action::emit(Event::PlayShuffled(id))` instead
    of the current no-op.
- **MainScreen maps artists' emit** (via `Action.handle(.., on_emit)`):
  `Task::perform(Compat::new(collect_shuffled_songs(client, id)))` →
  `Msg::Engine(EngineMsg::PlayQueue(tracks))`.
  `collect_shuffled_songs`: `get_artist(id)` → `artist.album` ids →
  `get_album(id)` → `song: Vec<Child>` (filter `!is_dir`) → flatten → shuffle.
- **Shuffle:** add `rand` for a Fisher–Yates shuffle (only new dep in Facet 1).
  Shuffle lives in the caller for now, not the engine.

### Facet 1 verification

- `cargo build` + `cargo clippy -- -D warnings` clean.
- Log in → Artists → hit shuffle: log shows `PlayQueue(n tracks)`; the bar shows
  a mock track, position advances each tick, tracks auto-advance through the
  queue; pause/resume/next/stop all change the bar state correctly. This proves
  the whole intent→event→coordinator→view pipe before audio exists.

## Facet 2 — Real playback engine (adds `rodio = "0.22"`, `libasound2-dev`)

`src/engine/player.rs` (`AudioPlayer`) + `src/engine/real.rs` (`RealEngine`):

- `AudioPlayer::open()`: `DeviceSinkBuilder::open_default_sink()` →
  `MixerDeviceSink` + `Player::connect_new(&mixer)`. Lives on the main thread
  (`!Send` device handle). Wraps: `play_bytes(Bytes)` (`Decoder::new(
  Cursor::new(bytes))` → `append`), `pause/resume/stop/skip`, `set_volume`,
  `try_seek(Duration)`, `position()`, `empty()`.
- `RealEngine` implements the same seam as the mock:
  - `PlayQueue`/`PlayNext`: set `current`/`loading`, bump `generation`, return
    a `Task::perform(Compat::new(fetch stream))` → `EngineMsg::BytesReady`.
    Discard stale generations.
  - `BytesReady { generation, Ok(bytes) }` where generation matches:
    `player.play_bytes(bytes)`, state = Playing, emit `PlaybackStarted`.
  - `tick`: `position = player.position()`; when `state == Playing &&
    player.empty()` → `TrackFinished(current)` → auto-advance: pop queue, fetch
    next, state = Loading. (No prefetch yet — Facet 4.)
  - `Err` → `EngineEvent::Error(String)` + back to `Stopped` (server JSON errors
    on `stream`, expired auth, network).
- Fetch (no cache yet): `client.stream(id, None, None, None, None)` wrapped in
  `async_compat::Compat`, whole file in memory (see Out of scope).
- Seek: best-effort `Player::try_seek`. rodio's wrapper chain
  (`speed/track_position/pausable/amplify/skippable/stoppable/periodic_access`)
  may not forward seek; if unsupported, fall back to stop + restart. Verify and
  document the outcome in `sound.md`.
- Swap: `MainScreen` constructs `Engine::Real(RealEngine::open(client))` — no
  UI changes beyond construction.

### Facet 2 verification

- Shuffle an artist → tracks actually stream and play; bar position is real;
  auto-advance works with a brief fetch gap between tracks; pause/resume/stop/
  next work; errors surface as `EngineEvent::Error` (kill the server mid-play to
  test).

## Facet 3 — Disk cache with LRU + `settings.toml` (no new deps)

- **Config:** `src/config.rs` gains `Settings` (sibling of `Credentials`),
  loaded from `config_dir()/settings.toml`:
  ```toml
  cache_max_size_mb = 512
  ```
  → `CacheConfig { max_size_bytes: u64, cache_dir: PathBuf }`
  (`cache_dir()` already exists).
- **`src/engine/cache.rs`:**
  - `Cache { root: PathBuf, max_size_bytes: u64 }`, `Arc`-shareable
    (`Send + Sync`, plain file IO), namespaces `stream` / `cover`.
  - Keys: server ids are untrusted strings → hash before using as a path
    component.
  - Manifest `index.toml` (serde + `toml`, already a dep): `key →
    { namespace, size, last_accessed }`.
  - `get(namespace, key) -> Option<Bytes>`: touch `last_accessed`, rewrite
    manifest, return file bytes.
  - `put(namespace, key, &Bytes)`: temp file + atomic rename; update manifest;
    then **while total size > max, evict the least-recently-accessed entry**
    (delete file + drop from manifest).
  - `invalidate(namespace, key)`.
- **`src/engine/fetch.rs`:** `stream_track(client, cache, &Track)` —
  `cache.get("stream", id)` → on hit return; on miss `client.stream(...)` →
  `cache.put` → return.
- **Corruption fallback:** if `Decoder` fails on cached bytes, the engine
  `invalidate`s + refetches once (`real.rs` retries with a per-track attempt
  count).
- Manifest rewrite per get/put is fine at hundreds of entries; revisit only if
  the cache grows huge.

### Facet 3 verification

- Play a queue once → cache dir fills; replay the same artist with the server
  unreachable → plays from disk (proves cache hit path).
- Set `cache_max_size_mb` tiny (e.g. 5) → oldest entries evict as new tracks
  stream; total stays under the cap.
- Corrupt a cached file (truncate it) → plays anyway after one refetch.

## Facet 4 — Extras (later, one at a time)

Prefetch next track into a one-slot buffer while playing (near-gapless; cache
already covers repeat plays); scrobbling (`PlaybackStarted` →
`client.scrobble(id, now, false)` "now playing", `TrackFinished` →
`submission = true`, threshold notes — e.g. ≥50% or skip-counts); downloads
(`client.download` through the cache, `download` namespace); cover-cache
migration (artists screen fetches via cache); player screen; transcoding
(`maxBitRate`/`format`); cache eviction tuning.

## Changes to existing files

| File | Change |
| --- | --- |
| `Cargo.toml` | Facet 1: `rand`. Facet 2: `rodio = "0.22"` |
| `src/lib.rs` | `pub mod engine;` |
| `src/engine/mod.rs` | types + `Engine` enum + `EngineError` (Facet 1) |
| `src/engine/mock.rs` | mock engine (Facet 1) |
| `src/engine/player.rs` | `AudioPlayer` (Facet 2) |
| `src/engine/real.rs` | real engine (Facet 2) |
| `src/engine/cache.rs` | disk cache + LRU (Facet 3) |
| `src/engine/fetch.rs` | cache-aware fetching (Facet 3) |
| `src/screen/mod.rs` | engine field, msgs, subscription, coordinator, live bar |
| `src/screen/artists.rs` | `Emit` = `Event`, play-shuffled emits |
| `src/config.rs` | `Settings` + `CacheConfig` (Facet 3) |
| `sound.md` | rodio 0.22 API notes + seek findings (Facet 2) |

## Out of scope / future work

Transcoding/format negotiation, true gapless (rodio queue-based), streaming
decode (Symphonia `MediaSource`) to avoid holding whole files in memory, cache
LRU tuning/shared cover cache eviction, playlist persistence, seeking hardening,
Android/iOS output backends.

## Verification (whole plan)

Per-facet checks above; plus `cargo build` / `cargo clippy -- -D warnings` after
every facet, and a final offline replay test proving the disk cache.