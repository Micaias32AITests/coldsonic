# Placeholder screens 01 — detail, library views, settings

Status: planned. Replaces the remaining placeholder routes described in
`screen_01.md` §5.11 and the deferred phases 3, 5, and 6 of `ui_01.md`.

Applies to: `src/screen/artist_detail.rs`, `src/screen/album_detail.rs`,
`src/screen/albums.rs` (new), `src/screen/all_tracks.rs` (new),
`src/screen/settings.rs`, `src/screen/artists.rs`, `src/screen/mod.rs`,
`src/library/mod.rs`, `src/library/manager.rs`,
`src/library/offline/mod.rs`, and `src/library/offline/store.rs`.

## Goal

Replace the remaining placeholder screens in this order:

1. **Artist detail** — artist header plus their albums.
2. **Album detail** — album header plus its tracks.
3. **Settings** — panel behavior and future settings entries.
4. **Albums** — flat list of every album in the merged library.
5. **All Tracks** — virtualized list of every track.
6. **Playlists** — deliberately deferred; the sidebar entry stays a
   placeholder.

Artist and album detail are first because the manager already exposes the data
they need (`Manager::artist`, `Manager::album`) and because album detail gives
All Tracks a reusable track-row pattern.

`ui_01.md` phase 3 only says “implement `artist_detail.rs` + `album_detail.rs`”
and phase 5 only says “settings root”. Neither document specifies a visual
layout. This plan supplies those layouts.

## 1. Locked-in decisions

- **No `Screen` trait.** Every new screen is a plain state struct with inherent
  `init`/`update`/`view` functions, following `src/screen/mod.rs` and
  `AGENTS.md`.
- **Duplicate routes store their own state.** `SubScreen::ArtistDetail`,
  `SubScreen::AlbumDetail`, `SubScreen::Albums`, `SubScreen::AllTracks`, and
  `SubScreen::Settings` change from unit/id variants to state-carrying
  variants. Two artist-detail entries must not share one state instance.
- **Screens receive only what they read.**
  - Artist/album/albums/all-tracks screens take the manager directly in
    `init`/`update`; they do not need `Ctx` because they read no engine or
    now-playing state.
  - Settings takes `&UiSettings` and the live `panel_open` flag directly in
    `view`; it does not store parent-owned settings in its `State`.
- **Mutations flow upward as events.** Clicking an artist/album, playing a
  track, and changing a setting all return `Action::Emit(Event)`. `MainScreen`
  performs the navigation, engine command, or settings write.
- **Cover handles are screen-owned.** Each screen keeps its own cover state and
  builds `image::Handle` once when bytes arrive. No shared cover cache.
- **Library methods are added only when a screen needs them.**
  - Artist detail and album detail need no new manager methods.
  - Albums adds `Manager::albums()`.
  - All Tracks adds `Manager::tracks()`.
- **Playlists are out of scope.** There is no playlist model in the library
  seam. Phase 6 in `ui_01.md` says the sidebar button reserves the slot only,
  and `library_01.md` does not add playlists. The placeholder stays until a
  separate playlist plan exists.
- **No inline back buttons.** Back remains the existing stack pop
  (`Alt+Left`); this plan does not add per-screen navigation chrome.

## 2. Shared screen conventions

### Signatures

```rust
// artist_detail.rs and album_detail.rs
pub fn init(id: /* ArtistId or AlbumId */, manager: Arc<Manager>) -> (State, Task<Msg>);
pub fn update(state: &mut State, manager: &Arc<Manager>, msg: Msg) -> Action<Msg, Event>;
pub fn view(state: &State) -> Element<'_, Msg>;

// albums.rs and all_tracks.rs
pub fn init(manager: Arc<Manager>) -> (State, Task<Msg>);
pub fn update(state: &mut State, manager: &Arc<Manager>, msg: Msg) -> Action<Msg, Event>;
pub fn view(state: &State) -> Element<'_, Msg>;

// settings.rs
pub fn init() -> State;
pub fn update(state: &mut State, msg: Msg) -> Action<Msg, Event>;
pub fn view<'a>(
    state: &'a State,
    settings: &'a UiSettings,
    panel_open: bool,
) -> Element<'a, Msg>;
```

None of these screens has a `subscription`.

### Async results are stale-safe

Messages produced by an async load carry the id that was requested:

```rust
Msg::Loaded(ArtistId, Result<ArtistDetail, String>)
Msg::Loaded(AlbumId, Result<AlbumDetail, String>)
```

`update` ignores a result whose id no longer matches `State.id`. This covers
the case where the user pops a detail screen before its fetch finishes and then
opens a different artist/album. Cover results are keyed by `CoverId` and are
ignored when no matching row exists.

### Cover pattern

Reuse the artists-screen pattern:

- `CoverState { NotLoaded, Loading, Loaded(image::Handle), Failed }`.
- Build the handle once when bytes arrive, never in `view`.
- Row covers fetch at `96` px for a `48` px display.
- Detail-header covers fetch at `256` px for a `128` px display.
- Missing or failed art uses the muted `lucide::music` placeholder.

### Coordinator registration

Every new screen must be registered in all five places:

- `SubScreen` in `src/screen/mod.rs`;
- `screen::Msg`;
- `MainScreen::update`;
- `MainScreen::view_route`;
- `MainScreen::make_screen`.

The `SubScreen` variants become:

```rust
pub enum SubScreen {
    Artists(artists::State),
    ArtistDetail(artist_detail::State),
    AlbumDetail(album_detail::State),
    Albums(albums::State),
    AllTracks(all_tracks::State),
    Playlists, // still a placeholder
    Settings(settings::State),
}
```

Add these coordinator messages:

```rust
ArtistDetailMsg(artist_detail::Msg),
AlbumDetailMsg(album_detail::Msg),
AlbumsScreenMsg(albums::Msg),
AllTracksScreenMsg(all_tracks::Msg),
SettingsScreenMsg(settings::Msg),
```

Dispatch follows the existing scoped-borrow pattern: borrow the last entry,
match the expected `SubScreen` variant, build the screen's arguments, call its
`update`, then handle the owned `Action` after the borrow ends. A message for a
screen that is no longer on top is dropped.

## 3. Phase 1 — Artist detail

### Data

`Manager::artist(&ArtistId) -> Result<ArtistDetail, LibraryError>` already
returns the artist and its albums, with ids and covers already prefixed.

### State and messages

```rust
pub struct State {
    id: ArtistId,
    artist: Option<Artist>,
    albums: Vec<AlbumRow>,
    loaded: bool,
    error: Option<String>,
    cover: CoverState,
}

struct AlbumRow {
    album: Album,
    cover: CoverState,
}

pub enum Msg {
    Loaded(ArtistId, Result<ArtistDetail, String>),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    AlbumClicked(AlbumId),
    Shuffle,
}

pub enum Event {
    AlbumClicked(AlbumId),
    PlayShuffled(ArtistId),
}
```

`init` starts `manager.artist(&id)` and returns immediately with
`artist = None`, `loaded = false`.

On `Loaded`:

- ignore mismatched ids;
- store the artist and build one `AlbumRow` per album;
- mark every album with a cover as `Loading` and spawn one fetch per unique
  `CoverId`;
- spawn the artist-header cover fetch if `artist.cover` is `Some`;
- set `loaded = true`.

On `CoverLoaded`, update the matching album row or the header cover.

`AlbumClicked` emits `Event::AlbumClicked`; `Shuffle` emits
`Event::PlayShuffled(self.id.clone())`.

### Layout

```
┌────────────────────────────────────────────────────┐
│ ┌────────┐  Artist Name                 [Shuffle] │
│ │ cover  │  N albums                             │
│ └────────┘                                         │
├────────────────────────────────────────────────────┤
│ [cover]  Album Name          Artist      12 songs  │
│ [cover]  Album Name          Artist       8 songs  │
│ ...                                                │
└────────────────────────────────────────────────────┘
```

- Header: 128 px square cover, artist name at size 20–24, album count at size
  12 and muted, and a `lucide::shuffle` button when the artist has albums.
- Album list: virtualized with `lazy_list` under a fixed header row, with
  columns
  - 48 px cover,
  - album name as a `button` that emits `Msg::AlbumClicked`,
  - artist name (muted; the artist line is useful for merged libraries),
  - `N songs`.
- `Msg::Scrolled` updates `visible`; covers are fetched only for the visible
  window (the header cover is always included).
- Empty state after loading: centered muted “No albums”.
- Loading: centered “loading...”, matching the artists screen.
- Error: centered muted error text with the same table absent.

### Wiring

- `artists::Msg::ArtistClicked` currently only logs. Add
  `artists::Event::ArtistClicked(ArtistId)` and change the handler to
  `Action::emit(...)`.
- `MainScreen::handle_artists_event` pushes `Route::ArtistDetail(id)`.
- `Route::ArtistDetail` constructs `artist_detail::init(id, manager.clone())` in
  `make_screen`.
- `ArtistDetail` event handling in `MainScreen`:
  - `AlbumClicked(id)` pushes `Route::AlbumDetail(id)`;
  - `PlayShuffled(id)` reuses the existing `collect_shuffled_songs` task.

## 4. Phase 2 — Album detail

### Data

`Manager::album(&AlbumId) -> Result<AlbumDetail, LibraryError>` already returns
the album and its songs in track order, with ids and covers prefixed.

### State and messages

```rust
pub struct State {
    id: AlbumId,
    album: Option<Album>,
    songs: Vec<Track>,
    loaded: bool,
    error: Option<String>,
    cover: CoverState,
}

pub enum Msg {
    Loaded(AlbumId, Result<AlbumDetail, String>),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    PlayAll,
    PlayTrack(Track),
}

pub enum Event {
    PlayQueue(Vec<Track>),
    PlayTrack(Track),
}
```

On `Loaded`:

- ignore mismatched ids;
- store the album and songs;
- fetch the album cover (256 px) when present;
- set `loaded = true`.

`PlayAll` emits `Event::PlayQueue(self.songs.clone())` when songs is non-empty.
`PlayTrack(track)` emits `Event::PlayTrack(track)`.

### Layout

```
┌──────────────────────────────────────────────────────┐
│ ┌──────────┐  Album Name                             │
│ │  cover   │  Artist · N songs · 42:13    [Play album]│
│ └──────────┘                                         │
├──────────────────────────────────────────────────────┤
│ #   Title                         Artist     03:45 ▶ │
│ 1   ...                                              │
│ 2   ...                                              │
└──────────────────────────────────────────────────────┘
```

- Header: 128 px cover, album name at size 20–24, then a muted metadata line:
  artist, song count, total duration (sum of track durations). Omit optional
  pieces cleanly instead of leaving dangling separators.
- `Play album` button emits `Msg::PlayAll`.
- Track list: `lazy_list` under a fixed header row, with columns
  - track number (`track_no`, blank when absent),
  - title as a `button` that emits `Msg::PlayTrack(track.clone())`,
  - artist (muted),
  - duration via `fmt_duration`,
  - a small `lucide::play` button that emits the same `Msg::PlayTrack`.
- Empty state after loading: centered muted “No songs”.
- Loading and error states match artist detail.

### Wiring

- `MainScreen::handle_album_detail_event`:
  - `PlayQueue(tracks)` → `Msg::Engine(EngineMsg::PlayQueue(tracks))`;
  - `PlayTrack(track)` → `Msg::Engine(EngineMsg::PlayNow(track))`.
- Both artist detail and the future Albums screen push `Route::AlbumDetail`.
- `Route::AlbumDetail` constructs `album_detail::init(id, manager.clone())`.

## 5. Phase 3 — Settings

### Scope

The first settings root contains only controls backed by the existing
`UiSettings` plus panel state. Library management, themes, and other future
entries are disabled placeholders with “coming soon” labels.

### State and messages

`State` has no local data yet; it exists so the screen follows the same shape
as the others and can grow later.

```rust
pub struct State;

pub enum Msg {
    PanelStartupSelected(PanelStartup),
    PanelScreenSelected(PanelScreenConfig),
    SetPanelOpen(bool),
}

pub enum Event {
    SetPanelStartup(PanelStartup),
    SetPanelScreen(PanelScreenConfig),
    SetPanelOpen(bool),
}
```

`update` only maps each `Msg` to the matching `Event`.

### Layout

```
Settings

Right panel
  Startup behavior     [Remember last session  v]
  Default screen       [Queue                  v]
  [x] Show right panel

Appearance
  Theme                System (coming soon)   [disabled]

Libraries
  Manage libraries     Coming soon            [disabled]

Settings are saved automatically.
```

- `pick_list` for the two config enums. Add `Display` impls in `src/config.rs`
  for `PanelStartup` and `PanelScreenConfig`:
  - `PersistState` → “Remember last session”
  - `StartClosed` → “Always start closed”
  - `StartOpened` → “Always start open”
  - `Queue` → “Queue”
  - `NowPlaying` → “Now playing”
- `checkbox` for the live panel open state.
- Muted, disabled rows for future entries.
- `view` takes `settings: &UiSettings` and `panel_open: bool`; it does not read
  `MainScreen` directly.

### Coordinator behavior

`MainScreen::handle_settings_event`:

- `SetPanelStartup(value)` stores it in `ui_settings.panel_startup` and saves
  immediately. It applies on the next launch.
- `SetPanelScreen(value)` updates both `ui_settings.panel_screen` and the live
  `panel_screen`, saves, and refreshes the panel. Reuse the existing
  `Msg::SwitchPanel` behavior where possible.
- `SetPanelOpen(open)` sets `panel_open`, opens/closes the pane through the
  existing pane helpers, saves, and refreshes the panel when opening.

Extract a small `set_panel_open(&mut self, open: bool) -> Task<Msg>` helper and
use it from both `Msg::TogglePanel` and the settings event so the two paths
cannot drift.

Existing sidebar behavior already pushes Settings over the current stack and
resets an existing Settings entry on re-click. Change the reset to
`SubScreen::Settings(settings::init())`.

## 6. Phase 4 — Albums

### Library API

Add album listing to the seam:

```rust
impl Manager {
    pub async fn albums(&self) -> Result<Vec<Album>, LibraryError>;
}

impl Library {
    pub async fn albums(&self) -> Result<Vec<Album>, LibraryError>;
}

impl OfflineLibrary {
    pub async fn albums(&self) -> Result<Vec<Album>, LibraryError>;
}

impl MemoryStore {
    pub(super) async fn albums(&self) -> Vec<Album>;
}
```

`MemoryStore` already keeps `albums: HashMap<AlbumId, Album>`; return cloned
values. `Library` delegates. `Manager::albums` concatenates each enabled
library, calls `re_prefix_album`, and sorts by album name case-insensitively,
then artist name for ties.

### Screen

```rust
pub struct State {
    rows: Vec<Row>,
    loaded: bool,
    error: Option<String>,
}

struct Row {
    album: Album,
    cover: CoverState,
}

pub enum Msg {
    Loaded(Vec<Album>),
    LoadFailed(String),
    AlbumClicked(AlbumId),
    CoverLoaded(CoverId, Result<Bytes, ()>),
}

pub enum Event {
    AlbumClicked(AlbumId),
}
```

- On `Loaded`, build rows, mark covers `Loading`, and spawn one fetch per unique
  cover id in the initial visible window. `Msg::Scrolled` narrows the window on
  scroll. This mirrors the artists list.
- `AlbumClicked` emits `Event::AlbumClicked`.
- `view` is a `lazy_list` under a fixed header row with columns: 48 px cover,
  album name as a navigation button, artist, `N songs`, and duration.
- Empty state: “No albums found”. Loading: “loading...”. Failure: muted error
  text.

### Wiring

- `Route::Albums` constructs `albums::init(manager.clone())`.
- `MainScreen::handle_albums_event` pushes `Route::AlbumDetail(id)`.

## 7. Phase 5 — All Tracks

### Library API

```rust
impl Manager {
    pub async fn tracks(&self) -> Result<Vec<Track>, LibraryError>;
}

impl Library {
    pub async fn tracks(&self) -> Result<Vec<Track>, LibraryError>;
}

impl OfflineLibrary {
    pub async fn tracks(&self) -> Result<Vec<Track>, LibraryError>;
}

impl MemoryStore {
    pub(super) async fn tracks(&self) -> Vec<Track>;
}
```

`MemoryStore` already keeps `tracks: HashMap<TrackId, Track>`; return cloned
values. `Library` delegates. `Manager::tracks` concatenates each enabled
library, calls `re_prefix_track`, and sorts by title case-insensitively, then
artist, album, track number, and id for stable ties.

### Screen

```rust
const ROW_HEIGHT: f32 = 56.0;
const COVER_DISPLAY: f32 = 48.0;
const COVER_PX: u32 = 96;
const INITIAL_ROWS: usize = 32;
const OVERSCAN: usize = 3;

pub struct State {
    rows: Vec<Track>,
    covers: HashMap<CoverId, CoverState>,
    visible: Range<usize>,
    loaded: bool,
    error: Option<String>,
}

pub enum Msg {
    Loaded(Vec<Track>),
    LoadFailed(String),
    Scrolled(iced::widget::scrollable::Viewport),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    Play(Track),
}

pub enum Event {
    PlayTrack(Track),
}
```

- Use `crate::widget::lazy_list::lazy_list(&state.rows, ROW_HEIGHT, ...)` inside
  the view. The list owns the scrollable; put a fixed header row above it.
- `Msg::Scrolled` updates `visible` with the shared
  `lazy_list::visible_range` helper (clamped offset, viewport height,
  overscan), which every virtualized list uses.
- After `Loaded`, set `rows`, seed `visible` to `0..INITIAL_ROWS`, and run the
  same cover-refresh logic as the queue screen: keep only covers in the current
  window, skip ids already `Loading`/`Loaded`/`Failed`, spawn 96 px fetches for
  the rest. `Scrolled` also triggers refresh.
- `CoverLoaded` replaces the matching entry; unknown/pruned ids are ignored.
- `Play(track)` emits `Event::PlayTrack(track)`.

### Layout

```
┌────────────────────────────────────────────────────────────┐
│ Title                          Artist      Album    03:45   │
├────────────────────────────────────────────────────────────┤
│ [art]  Title                   Artist      Album    03:45 ▶ │
│ [art]  Title                   Artist      Album    04:12 ▶ │
│ ...                                                        │
└────────────────────────────────────────────────────────────┘
```

- Fixed header row with muted labels.
- Each 56 px row: 48 px cover or muted music icon, title
  (`Wrapping::None`, clipped), artist, album, duration, and a play button.
- Column widths should be shared between the header and rows so the columns
  line up: cover fixed 48, title `FillPortion(4)`, artist `FillPortion(3)`,
  album `FillPortion(3)`, duration fixed, play fixed.
- Empty state: “No tracks found”. Loading: “loading...”. Failure: muted error
  text.

### Wiring

- `Route::AllTracks` constructs `all_tracks::init(manager.clone())`.
- `MainScreen::handle_all_tracks_event` maps `PlayTrack(track)` to
  `Msg::Engine(EngineMsg::PlayNow(track))`.

## 8. Playlists — deferred

No playlist work in this plan.

Reasons:

- `library_01.md` has no playlist type or manager method.
- `ui_01.md` §9 says “Playlist screens' contents ... the sidebar button reserves
  the slot only.”
- Adding an empty playlist screen would be a second placeholder with no data
  path.

Keep `Route::Playlists` and the sidebar button. The route continues to render
the existing “Playlists” placeholder. A future `playlists_01.md` must define:

- a playlist record type and stable ids in the library seam;
- manager/provider methods for listing and reading playlists;
- whether playlists are local-only, provider-specific, or both;
- create/rename/delete and track add/remove flows (all events upward).

## 9. Library tests

Add tests alongside the existing manager/offline tests:

- `MemoryStore::albums()` returns every scanned album; `tracks()` returns every
  scanned track.
- `Manager::albums()` merges two enabled fixture libraries, prefixes album ids
  and cover ids, and sorts by name case-insensitively.
- `Manager::tracks()` merges two enabled fixture libraries, prefixes track and
  cover ids, and sorts deterministically.
- A disabled library contributes neither albums nor tracks.
- `Library::Offline` delegates both new methods.

Existing fixtures under `tests/fixtures/music/manager-a` and `manager-b` are
enough; add assertions to the existing manager tests rather than new fixture
trees unless a missing case requires one.

## 10. Implementation order

1. **Artist detail** — state, layout, cover fetching, `Artists` click event,
   `MainScreen` route push. Keep the tree compiling.
2. **Album detail** — state, layout, play events, navigation from artist
   detail, `MainScreen` engine mapping.
3. **Settings** — `Display` impls, settings layout, events, `set_panel_open`
   helper, sidebar reset-to-settings.
4. **Albums** — manager/provider/store `albums`, screen, route push.
5. **All Tracks** — manager/provider/store `tracks`, lazy-list screen with
   windowed covers, play event.
6. **Playlists** — no code change; keep the placeholder.

Run the checks after each phase, not only at the end.

## 11. Verification

Automated:

- `cargo fmt`
- `cargo check`
- `cargo check --features dev`
- `cargo test`
- `cargo clippy --all-targets --features dev -- -D warnings`

Manual:

- Artists → click an artist → artist detail loads, header cover or placeholder
  appears, albums list loads, album covers appear once.
- Click an album from artist detail → album detail loads, header metadata and
  track list are correct; `Play album` starts the album queue; a row play
  button starts that track.
- Pop with `Alt+Left` returns to the previous detail/list and preserves the
  previous screen's state.
- Two different artist-detail entries opened in sequence do not share state.
- Settings: change startup behavior, default panel screen, and show/hide the
  panel; changes apply immediately where applicable, persist to
  `settings.toml`, and survive a restart according to the startup policy.
- Sidebar Settings pushes over the current screen; re-clicking Settings resets
  the settings screen instead of nesting another copy.
- Sidebar Albums → album list → album detail.
- Sidebar Tracks → all tracks; scrolling stays smooth, covers load for newly
  visible rows, and stale cover results do not appear on the wrong row.
- Empty/loading/failure states for each new screen.
- Playlists still shows the placeholder and does not crash.

## 12. Out of scope / future

- Playlist model, UI, and persistence.
- Library management inside Settings (enable/disable/edit/remove libraries);
  `library_01.md` keeps that future work.
- Themes, visualizer, lyrics, equalizer, and other settings entries.
- Sorting/filtering/search on Albums or All Tracks.
- Inline back buttons, context menus, and multi-select.
- An album-detail shuffle action; only `Play album` is planned for now. Artist
  shuffle reuses the existing artist-shuffle path.
