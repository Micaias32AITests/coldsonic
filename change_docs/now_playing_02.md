# Now-playing bar v2 — richer bottom bar

Applies to: `src/engine/mod.rs`, `src/engine/mock.rs`, `src/screen/mod.rs`.

## Goal

Upgrade the compact now-playing bar from a flat readout (placeholder cover,
title/artist, raw `m:ss / m:ss` text, play/pause, skip-next) to a real player
bar: actual cover art, a seek slider with elapsed/total readouts, volume
control (slider + mute toggle), and a full transport (prev / play-pause / next
/ stop).

Scope is bar-only. A dedicated player screen is later work; the cover-fetch
state, seek-draft logic, and transport row are designed to be reused by it.

## Decisions

- **Volume is engine state.** The engine owns playback state; the bar reads it
  from the pushed `NowPlaying` snapshot. Add `volume: f32` (0.0–1.0) to it
  rather than keeping volume in the UI.
- **`Prev` needs history.** `Next`/auto-advance already model a "up next"
  queue; add a `history` deque (newest last) for previous tracks. Standard
  player semantics: if position > 3 s, `Prev` restarts the current track;
  otherwise it steps back into history.
- **Seek draft to avoid tick fights.** The 250 ms tick overwrites the snapshot
  position every frame. While the seek slider is being dragged, keep a draft
  position in `MainScreen`; only send `EngineMsg::Seek` on release.
- **Volume updates immediately.** `SetVolume` needs to reach the bar while
  dragging, but is harmless (unlike seek), so it fires continuously on change
  and the mock pushes `UpdateNowPlaying` to refresh the snapshot.
- **Cover fetch reuses the artists-screen pattern.** Track (a Subsonic `Child`)
  already carries `cover_art`. Fetch via `client.get_cover_art(id, Some(96))`
  (2× the 48 px display for HiDPI); build the `image::Handle` once on arrival
  and reuse it every frame so iced's image cache is not defeated (see the
  comment in `artists.rs`). Key results by track id and drop stale ones.
- **No new dependencies.**

## Changes

### 1. `src/engine/mod.rs`

- Add `Prev` to `EngineMsg` (only missing transport command; `Seek`,
  `SetVolume`, `Next`, `Pause/Resume/Toggle`, `Stop` already exist).
- Add `pub volume: f32` to `NowPlaying`.

### 2. `src/engine/mock.rs`

- New fields: `history: VecDeque<Track>` (newest last), `volume: f32` (default
  `1.0`).
- `PlayQueue` / `Stop`: reset `history`.
- `Next`, `PlayNext`, `advance` (auto-advance): before switching current,
  push the outgoing current track onto `history`.
- New `EngineMsg::Prev` arm:
  - `position > Duration::from_secs(3)` → restart current: position 0,
    state Playing, emit `PlaybackStarted(current)`.
  - else if `history` non-empty → push current onto `queue` front, pop
    `history` back, set as current, position 0, state Playing, emit
    `PlaybackStarted(prev)`.
  - else → no-op.
- `SetVolume(v)`: store `v.clamp(0.0, 1.0)`, push `UpdateNowPlaying` so the
  bar refreshes while dragging.
- `now_playing()`: include `volume: self.volume`.

### 3. `src/screen/mod.rs`

New `MainScreen` fields:

- `cover: Option<image::Handle>` — current track's art.
- `seeking: Option<f32>` — draft seek position (seconds).
- `last_volume: f32` — remembered volume for mute toggle, default `1.0`.

New messages:

- `CoverLoaded(String, Result<Bytes, ()>)` — track id + fetch result.
- `SeekDragged(f32)` — store draft in `seeking`; `Action::none()`.
- `SeekReleased` — take draft, clear `seeking`, then
  `Action::task(Task::done(Msg::Engine(EngineMsg::Seek(Duration::from_secs_f32(d)))))`.

`apply_engine_event`:

- On `PlaybackStarted(track)`: clear `cover` and `seeking`; if
  `track.cover_art.is_some()`, spawn
  `Task::perform(Compat::new(client.get_cover_art(id, Some(COVER_PX))), ...)`
  mapping to `Msg::CoverLoaded(id, result)` (mirror `artists.rs`).
- On `Stopped`: clear `cover` and `seeking`.

`update` additions:

- `CoverLoaded(id, result)`: ignore unless the current track id matches;
  set `cover = Loaded(Handle)` on `Ok`, `None` (fallback shown) on `Err`.
- `SeekDragged(v)`: `self.seeking = Some(v)`.
- `SeekReleased`: as above.

Constants (mirroring `artists.rs`): `COVER_DISPLAY = 48.0`, `COVER_PX = 96`.

`view_now_playing` rewrite — three-section bar:

- **Left:** 48×48 cover (`image(handle)`), or a `lucide::music()` fallback in a
  muted container while loading / when the track has no art. Beside it a
  column: title (default size) over artist (size 12, dim) — as today.
- **Center** (`.width(Fill)`, contents centered):
  - Transport row: `skip_back` | `play`/`pause` (by `np.state`, as today) |
    `skip_forward` | `square` (stop).
  - Progress row: `fmt_duration(elapsed)` + seek slider + `fmt_duration(total)`.
    - Slider: `slider(0.0..=total_secs, seeking.unwrap_or(position_secs),
      Msg::SeekDragged)`, `.on_release(Msg::SeekReleased)`.
    - Guard degenerate ranges (`total_secs.max(1.0)`) and clamp position.
- **Right:** mute toggle button (`lucide::volume_x` when muted / `volume_2`
  otherwise; toggle sends `SetVolume(0.0)` and stores `last_volume`, or
  `SetVolume(last_volume)` to unmute) + volume slider
  `slider(0.0..=1.0, np.volume, Msg::Engine(EngineMsg::SetVolume))`
  (~100 px wide).

Keep the existing "Nothing playing" empty-state text. Give the bar
`width(Fill)` with light padding, as today. `fmt_duration` unchanged.

## Icon note

Existing code uses `lucide::skip_forward()`, `lucide::shuffle()`, `lucide::play()`,
`lucide::pause()`. Plan uses `skip_back`, `square`, `volume_x`, `volume_2`,
`music` — verify these names compile against iced_fonts 0.3 at build time; they
are standard lucide glyph names (snake_case of the icon name).

## Verification

1. `cargo build` and `cargo clippy -- -D warnings` clean.
2. Manual: log in → shuffle an artist →
   - real cover art appears in the bar;
   - position advances each tick; elapsed/total readouts track it;
   - dragging the seek slider previews position and the 250 ms tick does not
     yank the thumb; releasing seeks (position jumps and keeps advancing);
   - volume slider + mute toggle update the bar immediately;
   - prev (restart vs. back-track), next, stop all behave;
   - track auto-advance still works.
3. Rapid track changes don't leave a stale cover (stale `CoverLoaded` ignored
   by track id).

## Future (out of scope)

Dedicated player screen — reuse `cover` fetch state, `seeking` draft, and the
transport/progress rows.