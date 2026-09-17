# Artist screen — table layout with covers and play-shuffled button

Applies to: `src/screen/artists.rs`

## Goal

Turn the plain list of artists into a proper table so more information can be
shown at a glance, and start paving the way for playback from the artist
screen.

- Replace the `widget::Column` list with `widget::table`.
- Show each artist's cover art on the left, loaded asynchronously.
- Add a "play all songs shuffled" button on the right of each row.
  Pressing it only *sends a message* for now — the message is a no-op.

## Current behavior

- `view()` renders one full-width `button(artist.name)` per artist inside a
  `Column`, wrapped in a `scrollable`.
- State is `{ client: Arc<Client>, artists: Vec<ArtistId3> }`.
- No cover art, no per-row actions beyond the clickable name
  (`Msg::ArtistClicked`), no loading indicator.
- `Emit = Never`, `InitData = Arc<Client>`.

## Changes

### 1. Table layout

Use `iced::widget::table::table` with columns built via `table::column`:

```rust
table(
    [
        table::column(text(""), |row: &Row| cover_cell(&row)),      // cover
        table::column(text("Artist"), |row: &Row| text(&row.artist.name)),
        table::column(text("Albums"), |row: &Row| text(&/* album_count */)),
        table::column(text(""), |row: &Row| shuffle_button(&row)),   // action
    ],
    self.rows.iter().cloned(),
)
.width(Fill)
.height(Fill)
```

Notes / gotchas:

- The table widget requires `T: Clone` (rows are cloned to build each cell), so
  the row type used as `T` must be `Clone`.
- `table::column`'s view closure receives the row item by value and must return
  something `Into<Element>`.
- Keep the table inside a `scrollable` if row count can exceed the viewport.
- The `column!` list approach disappears; drop the now-unused imports.
  Consider switching the file to `use crate::prelude::*` (table helpers are
  re-exported through `iced::widget::*`).

### 2. Row model with cover state

Introduce a row type instead of storing bare `Vec<ArtistId3>`:

```rust
#[derive(Debug, Clone)]
struct Row {
    artist: ArtistId3,
    cover: CoverState,
}

#[derive(Debug, Clone)]
enum CoverState {
    NotLoaded,
    Loading,
    Loaded(bytes::Bytes), // get_cover_art returns Bytes; clone is cheap (ref-counted)
    Failed,
}
```

`State` becomes:

```rust
struct State {
    client: Arc<Client>,
    rows: Vec<Row>,
}
```

### 3. Async cover loading

- `opensubsonic::Client::get_cover_art(&id, size: Option<i32>)` returns
  `Result<Bytes, Error>`; wrap the future in `async_compat::Compat` like the
  existing `get_artists` call so it runs on the smol executor.
- When `ArtistsLoaded` arrives, build `rows` and immediately mark every artist
  that has `cover_art: Some(...)` as `Loading`, spawning one
  `Task::perform(..., |res| Msg::CoverLoaded(id, res))` per artist.
- New message (task result):
  `Msg::CoverLoaded(String /* artist id */, Result<Bytes, opensubsonic::Error>)`
  -> update the matching row's `CoverState`.
- `view()` renders:
  - `Loading`/`NotLoaded` -> a `space()`/placeholder (fixed square size).
  - `Loaded(bytes)` -> `image(iced::widget::image::Handle::from_bytes(bytes))`
    fixed-size (e.g. `48.0`).
  - `Failed` -> empty space (or placeholder icon).
- Encode as `image::Handle` each frame from the cached `Bytes`; no network work
  in `view()`.

Optional hardening (can be follow-ups, see Non-goals): only start loading the
first N covers per frame to avoid a burst of requests.

### 4. Play-shuffled button (no-op)

- New message: `Msg::PlayShuffled(String /* artist id */)`.
- `update()` handles it with a no-op (`{}`) plus a `println!`/log stub for now.
- `view()` adds a small icon button in the last column:

```rust
button(lucide::shuffle().size(16)).on_press(Msg::PlayShuffled(artist.id.clone()))
```

  using the already-bundled `iced_fonts` lucide set.

## New state / messages summary

| Item | Change |
| --- | --- |
| `State.artists: Vec<ArtistId3>` | -> `State.rows: Vec<Row>` |
| `Msg::ArtistsLoaded` | unchanged purpose |
| `Msg::ArtistClicked(String)` | kept |
| `Msg::CoverLoaded(String, Result<Bytes, Error>)` | new |
| `Msg::PlayShuffled(String)` | new, no-op handler |

## Out of scope / future work

- Actually building a shuffled queue: fetching an artist's songs will need a
  follow-up. The library exposes `get_artist(id) -> ArtistWithAlbumsId3`
  (albums) then song children per album (`get_album`/`Child`), which can feed a
  future shuffle-on-client-side implementation.
- Cover cache eviction / cross-screen shared image cache.
- Table styling (separators, row height, hover) and sorting.

## Verification

- `cargo run`: artist list renders as a table with cover thumbnails (once
  covers load on the right side, an 'Albums' count, and a shuffle button).
- Clicking shuffle prints a stub log line; nothing else happens.
- `cargo clippy -- -D warnings` clean (resolve the "never read" `client`/cover
  warnings introduced/remaining).