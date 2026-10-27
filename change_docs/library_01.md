# Media libraries — manager seam, offline provider, subsonic restore

Applies to: `src/library/*` (new), `src/screen/*`, `src/engine/*`,
`src/config.rs`, `src/main.rs`, `Cargo.toml`.

## Goal

coldsonic currently only speaks Subsonic, and the server is gone. Introduce a
**library** abstraction so songs can come from many kinds of sources — offline
files today, subsonic next, smb/youtube someday — and move every `opensubsonic`
type/leak behind that seam. A top-level **manager** is the only thing the UI
holds: it routes queries to the enabled libraries, merges their results, and
owns the single cache seam where sqlite will later slot in. This doc covers
three facets:

1. **Core seam + manager + offline provider** — neutral types, the `Library`
   provider enum, the `Manager`, and a filesystem-scanning offline provider
   (tag-based and folder-based, chosen by config).
2. **Config + set-library screen + rewire** — `libraries.toml`, a reusable
   `library_setup` screen (select / create / open), and the full UI + engine
   switch from `Arc<Client>` to `Arc<Manager>`. End result: the whole app works
   with zero network.
3. **Subsonic provider** — the same enum variant wrapping `opensubsonic::Client`
   again, restoring server support with no further UI changes.

Facets 1–2 are separate, verifiable milestones; Facet 3 waits until the server
is back.

## Locked in decisions

- **The manager is the only seam the UI touches.** `MainScreen` owns
  `Arc<Manager>`; child screens receive `&Arc<Manager>` through `Ctx` or direct
  parameters and never store it. Screens ask it about
  artists/albums/tracks/covers (streams and scrobbles later). Providers never
  reach the screens, and nothing above the manager knows how many libraries
  exist or how they store data. This is the single chokepoint that makes
  everything else easy: multi-source merging in one place, and sqlite later
  without touching a single screen.
- **Enum, not trait, at the provider level.** `enum Library { Offline(..) }`
  mirrors the `Engine` enum (`src/engine/mod.rs`): derives `Debug + Clone` for
  iced, zero dyn/boxing machinery, and exhaustive matches mark every place a new
  provider touches. A dyn-safe trait would fight stable Rust (`async fn` in dyn
  traits) for no gain while the set of providers is small and known. The manager
  is a plain struct over a `Vec` of these enums.
- **Enable/disable is config, from day one.** `LibraryConfig` gains
  `enabled: bool` (default `true`). The manager opens only enabled entries;
  disabled ones are skipped entirely. Toggling is a config edit for now (the
  settings screen comes later) — but the model and the skip logic exist from
  Facet 1.
- **Merged artists, for now.** `Manager::artists()` concatenates the enabled
  libraries' artists and sorts by name (case-insensitive, stable for ties).
  Same-named artists from different libraries stay separate rows — they are
  distinct ids and distinct sources, and merging rows across providers is a
  display-policy question for later (see Future work). The merge policy lives
  _only_ in the manager, so when this changes (per-library tabs, a source
  filter, dedup-by-name) no provider or screen changes with it.
- **Ids are opaque newtypes, prefixed with the library identifier for routing.**
  `TrackId(String)`, `ArtistId(String)`, `AlbumId(String)`, `CoverId(String)` —
  deriving `Debug, Clone, PartialEq, Eq, Hash,
  Serialize, Deserialize`.
  Providers generate _unprefixed, provider-scoped_ ids internally; the manager
  stamps every id it returns with `{identifier}:` and strips the prefix when
  routing a query back down. A subsonic id can never be mistaken for a local
  path, and the prefix makes routing a pure string operation with no guessing.
- **Every registered library has an identifier (`id`).** The `{identifier}:`
  prefix is this — _not_ the display name (names aren't unique, aren't
  charset-constrained, and get renamed; the identifier is the permanent key).
  It's a stable `[a-z0-9-]+` string, unique among entries, part of
  `LibraryConfig`. Because it never changes and doesn't depend on config order
  or enablement, prefixed ids can be persisted by sqlite later with no config
  migration. Registration generates it automatically from the name — `slugify`:
  lowercase, any run of chars outside `[a-z0-9]` collapses to a single `-`,
  trimmed, empty → `unknown`, collisions get `-2`, `-3`, … — but the user can
  decide it themselves (see the form spec in Facet 2). Hand-edited configs must
  include `id`; a missing or duplicate identifier is a loud load/open error,
  matching the typo'd-root rule.
- **Sqlite slots in at the manager, above all providers.** The manager owns
  `cache: CacheStore` (`enum CacheStore { None }` today, same
  single-variant-enum pattern `Engine` uses). Later, `Store::Sqlite` caches
  metadata records — and cover/stream blobs — for _every_ provider in one place.
  There is deliberately **no** storage enum inside the offline provider: its
  in-memory index is the scan result (a fact, not a cache), so the offline
  provider needs no future-proofing of its own anymore.
- **All neutral types derive `Serialize`/`Deserialize`** so they double as
  persistence records when the sqlite cache lands; ids are stable across rescans
  (paths/names, not row numbers).
- **Scan runs at open.** `OfflineLibrary::open(root, mode)` does the blocking
  filesystem walk + tag reads inside `smol::unblock`, producing a fully indexed
  library. Every later query is a cheap in-memory read behind the same async
  signature (so genuinely async backends fit without caller changes), and the
  setup screen gets a natural loading state.
- **Offline scan modes are config, both supported from day one.**
  `scan = "tags"` (default): lofty reads embedded tags for
  title/artist/album/track-no and embedded pictures for covers.
  `scan = "folders"`: names come from the `Artist/Album/01 - Title.ext` path
  structure; duration is still read from file properties via lofty (a physical
  fact, not a metadata opinion); covers from `cover.jpg`/`folder.jpg` (or
  siblings like `Folder.jpg`) next to the album's files, else embedded art as a
  bonus.
- **Artists merge by name within the offline provider** in both modes, mirroring
  subsonic's model of a global artist entity (same-named folders/tags collapse
  into one artist). Across providers, see above.
- **`libraries.toml`** (sibling of `credentials.toml`, same TOML+serde pattern)
  holds N library entries with their `id` and `enabled` flag. The
  `LibraryConfig` type itself lives in `src/library/mod.rs` (the manager
  consumes it); `src/config.rs` only does file IO. `credentials.toml` is dropped
  in Facet 3; its job moves into the subsonic variant of the config.
- **The manager opens every enabled library up front** (at `Manager::open`,
  while the setup screen shows progress). One scan at startup beats hidden
  laziness; per-library lazy open is later if startup cost ever matters. If any
  enabled entry fails to open, `open` fails with an error naming it — a typo'd
  root should be loud, not a silently smaller library.
- **`library_setup.rs` is a reusable shell**, not a one-off. One screen with
  modes (select list → per-kind create form → opening progress). Facet 3 adds
  its subsonic login as another mode without touching the shell. The state
  machines and messages are fully implemented; the `AddOffline` form's `view`
  ships as a stub for manual implementation (spec below).
- **Only methods the UI calls today**: `artists`, `artist` (with albums),
  `album` (with songs), `cover`. Methods get added when a caller appears — same
  rule the engine docs use. `Track` stays opaque to the engine;
  `stream(&TrackId)` slots in later for real playback without touching screens.

## Facet 1 — core module + manager + offline provider (adds `lofty`, no UI changes)

```
src/library/
├── mod.rs        # neutral types, Library enum, LibraryError, LibraryConfig, ScanMode, slugify
├── manager.rs    # Manager + CacheStore + id prefixing + merge
├── offline/
│   ├── mod.rs    # OfflineLibrary: open() + async query methods → store
│   ├── scan.rs   # fs walk + lofty → ScanData (serde records)
│   └── store.rs  # MemoryStore built from ScanData
└── subsonic.rs   # Facet 3
```

### Neutral types (`src/library/mod.rs`)

```rust
pub struct TrackId(String);   // + ArtistId, AlbumId, CoverId (newtypes as above)

pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub duration: Duration,          // Child's Option<i64> secs → real Duration
    pub cover: Option<CoverId>,
}

pub struct Artist { pub id: ArtistId, pub name: String, pub album_count: u32 }
pub struct ArtistDetail { pub artist: Artist, pub albums: Vec<Album> }
pub struct Album  { pub id: AlbumId, pub name: String, pub artist: Option<String>,
                    pub song_count: u32, pub duration: Option<Duration>,
                    pub cover: Option<CoverId> }
pub struct AlbumDetail { pub album: Album, pub songs: Vec<Track> }
```

Counts (`album_count`, `song_count`) are plain `u32`, not `Option`: offline
always knows them (they come straight from the scan index). Subsonic's wire
fields are optional (`ArtistId3.album_count` / `AlbumId3.song_count` are
`Option<i64>`), so the conversion **falls back to `0` when the field is
absent and logs a `warn!`** carrying the artist/album id + name and which
field was missing — a server that doesn't report counts should be noticed,
not silently rendered as a real `0`.

`Library` is the _provider_ seam — an enum wrapper with delegating async methods
(ids unprefixed at this level):

```rust
#[derive(Debug, Clone)]
pub enum Library {
    Offline(offline::OfflineLibrary),
    // Facet 3: Subsonic(subsonic::SubsonicLibrary),
}

impl Library {
    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError>;
    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError>;
    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError>;
    pub async fn cover(&self, id: &CoverId, px: u32) -> Result<Bytes, LibraryError>;
}
```

Providers only ever return playable tracks — the `!is_dir` filter that lives in
`collect_shuffled_songs` today moves inside the subsonic provider in Facet 3.
Cover art comes back as `bytes::Bytes` (same as today's `get_cover_art`), so the
iced `image::Handle` path is unchanged.

`LibraryConfig` also lives here (serde; the manager consumes it, `config.rs`
only serializes it):

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LibraryConfig {
    Offline {
        name: String,
        /// Stable `[a-z0-9-]+` slug; the `{identifier}:` id prefix.
        /// Unique among entries; required in the config file.
        id: String,
        root: PathBuf,
        #[serde(default)]
        scan: ScanMode,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    // Facet 3: Subsonic { name, id, url, username, password, #[serde(default = "default_true")] enabled },
}
```

`pub fn slugify(name: &str) -> String` also lives here — pure, unit-tested:
lowercase; any run of chars outside `[a-z0-9]` → one `-`; trim `-`; empty →
`unknown`. Used by the create form's auto-identifier mode, and for `-2`/`-3`/…
collision suffixes when a generated slug is already taken.

`LibraryError` (thiserror, replacing `EngineError::Subsonic` in Facet 2):

```rust
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    NotFound(String),
    // Facet 3: #[error("subsonic error: {0}")] Subsonic(#[from] opensubsonic::Error),
}
```

### The manager (`src/library/manager.rs`)

```rust
pub struct Manager {
    /// Enabled entries only. Looked up by `LibraryConfig` identifier;
    /// that identifier is the id prefix. Vec, not map — order documents
    /// config order for debugging, and lookups are by string anyway.
    libraries: Vec<Opened>,
    cache: CacheStore,
}

struct Opened {
    config: LibraryConfig,
    library: Library,
}

/// The one cache seam, above every provider. Sqlite slots in here later
/// (metadata records + cover/stream blobs); nothing else in the codebase
/// needs a cache hook.
#[derive(Debug, Clone, Copy)]
pub enum CacheStore {
    None,
    // Future: Sqlite(sqlite::Cache) — rusqlite, data_dir()/library.sqlite
}
```

- `async fn open(configs: &[LibraryConfig]) -> Result<Manager, LibraryError>`:
  filter `enabled`, open each in order (offline entries via
  `OfflineLibrary::open`). Hard error naming the offending library on the first
  open failure — a typo'd root must be loud. Duplicate or malformed identifiers
  among the entries are rejected here too, before any scanning starts.
- `async fn artists(&self) -> Result<Vec<Artist>, LibraryError>`: concat all
  libraries' artists, prefix each id with `{identifier}:`, sort by name
  case-insensitively (stable for ties). The merge policy — and its eventual
  replacement — lives only here.
- `async fn artist/album/cover(...)`: split the id at the first `:`, find the
  library whose identifier matches, strip the prefix, call the provider, then
  **re-prefix every id in the returned structure** (albums inside
  `ArtistDetail`, songs' ids and covers inside `AlbumDetail`). Empty, unknown,
  or non-matching identifier → `LibraryError::NotFound`.
- Routing is pure string work — no provider is ever asked "is this yours?".
- Future methods (`stream`, `scrobble`, `refresh`) attach here, not on
  providers, so screens keep exactly one import.

### Offline provider (`src/library/offline/`)

```rust
pub struct OfflineLibrary {
    root: PathBuf,   // kept for rescan later; not read after open
    mode: ScanMode,
    store: MemoryStore,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    #[default]
    Tags,
    Folders,
}
```

- `open(root, mode) -> Result<Self, LibraryError>`: async; runs
  `scan::scan(&root, mode)` in `smol::unblock` (lofty is sync), builds the
  store. An empty music folder is not an error — open succeeds with an empty
  index (the UI shows an empty state, Facet 2).
- Audio extensions walked: mp3, flac, ogg, opus, m4a, wav (lofty-supported set;
  extendable in one array).
- **`scan.rs`** produces `ScanData { artists, albums, tracks, covers }` — plain
  serde records, no behavior:
  - `Tags`: walk files, read embedded tags (title/artist/album/track-no/
    duration); group artist-name → album-name; embedded picture → cover.
  - `Folders`: names from `Artist/Album/NN - Title.ext` path components;
    duration from file properties; folder-art candidates (`cover.jpg`,
    `Folder.jpg`, `folder.png`, …) → album cover, else embedded art.
  - Unprefixed ids are generated here and are stable across rescans: `TrackId` =
    path relative to root with normalized `/` separators, `ArtistId` = artist
    name, `AlbumId` = `artist\u{0}album`, `CoverId` = album path (folder art) or
    track path (embedded).
  - Files whose artist/album can't be determined (both modes) go to a fallback
    "Unknown Artist"/"Unknown Album" grouping rather than being dropped.
- **`store.rs`**: `MemoryStore::from(ScanData)` with query methods (async,
  taking `&self`): `artists()`, `artist(artist_id)`, `album(album_id)`,
  `album_songs(album_id)`, `cover_bytes(cover_id, px)`. Cover `px` is currently
  ignored (local files are returned as stored; resize comes later if needed) —
  the parameter keeps the signature identical to the subsonic provider's
  `get_cover_art(id, size)`. No enum wrapper: the future-proofing moved up to
  `Manager::cache`.

### Facet 1 verification

- `cargo build`, `cargo clippy -- -D warnings`, and `cargo test` clean.
- The repo's first unit tests, all over tiny checked-in fixture trees
  (`tests/fixtures/music/` — a couple of stub mp3s with tags written by the test
  via lofty, plus one `cover.jpg`; a second small tree for the manager):
  - tags mode: artist/album grouping from tags, track order, duration parsed,
    embedded art found;
  - folders mode: names parsed from paths, folder art found, duration from file
    properties;
  - id stability (same tree scanned twice → same ids), unknown-artist fallback,
    empty tree → empty index, `Library::Offline` delegates;
  - slugify: spaces/`:`/case collapse, trim, empty → `unknown`, collision
    suffixes (`-2`, `-3`);
  - manager: two fixture libraries (one enabled, one disabled) → disabled one
    not opened; two enabled → `artists()` merged+sorted with `alpha:`/`beta:`
    prefixed ids; `album`/`cover` routed by identifier and re-prefixed on the
    way out; unprefixed / unknown-identifier id → `NotFound`; duplicate
    identifiers among enabled entries → `open` error.

## Facet 2 — config + `library_setup.rs` + rewire (offline end-to-end)

### Config file IO (`src/config.rs`)

`libraries.toml` at `config_dir()`, one entry per library:

```toml
[[library]]
name = "Local"
id = "local"
kind = "offline"
root = "/home/you/Music"
scan = "tags"        # or "folders"; defaults to "tags" when omitted
enabled = true       # default when omitted
```

`load_libraries() -> Vec<LibraryConfig>` (missing file → empty list; entries
missing `id` → error naming the entry) and `save_libraries(&[LibraryConfig])`
(writes the file; used by the create form). `LibraryConfig`/`ScanMode` stay in
`src/library/mod.rs` — this module is only file IO.
`Credentials`/`credentials.toml` are dropped here — their only job (auto-login
seed) is superseded by the library list; subsonic creds return in Facet 3 inside
`libraries.toml`.

### Set-library screen (`src/screen/library_setup.rs`, replaces `login.rs`)

A reusable shell, not a one-off form:

```rust
pub struct State {
    libraries: Vec<LibraryConfig>,
    mode: Mode,
}

enum Mode {
    Select { error: Option<String> },
    AddOffline { name: String, id: String, auto_id: bool,
                 root: String, scan: ScanMode, saving: bool, error: Option<String> },
    Opening,   // Manager::open in flight
}

impl State {
    pub fn init() -> (Self, Task<Msg>);          // reads config in init
    pub fn update(&mut self, msg: Msg) -> Action<Msg, Arc<Manager>>;
    pub fn view(&self) -> Element<'_, Msg>;
}
```

The `Arc<Manager>` handoff is the screen's `Emit` event: `Action::emit(manager)`
on success, handled by `main.rs` as `Msg::LibraryReady`. The setup screen is a
root and does not receive `Ctx`.

- `Mode::Select`: one button per **enabled** library (any of them opens the
  whole merged set — they all lead to the same view); disabled entries listed
  greyed with a `(disabled)` label and no action (toggling is a config edit
  until the settings screen); an "Add local library" button. Clicking an enabled
  entry → `Mode::Opening` + `Task::perform(Manager::open(&libraries))` →
  `Emit(Arc<Manager>)` on success, back to `Select` with the error shown on
  failure. **Empty config, or no enabled entries (first boot / all disabled),
  lands directly in `AddOffline`.**
- `Mode::AddOffline` — the registration form. **State, messages, and logic are
  fully implemented; only `view` is a stub** (see below). Messages and behavior:
  - `NameEdit(s)`: store; when `auto_id` is on, `id = slugify(s)`.
  - `IdEdit(s)`: store; turn `auto_id` **off** (manual entry wins).
  - `AutoIdToggled(on)`: store; when turned on, `id = slugify(name)`
    immediately.
  - `RootEdit(s)`, `ScanToggled(mode)`: store.
  - `AttemptSave`: validate — name non-empty, root non-empty, `id` matches
    `[a-z0-9-]+`, `id` unique across existing entries — else fill `error`. On
    success: append `LibraryConfig::Offline { .. }`, `save_libraries`, enter
    `Opening`.
  - Layout for whoever implements the view (this is the spec):
    - a text field for the **name**;
    - below it, the **identifier field**, which live-updates to `slugify(name)`
      as the name is typed while auto mode is on;
    - to the **right** of the identifier field, a checkbox for "identifier is
      decided automatically" (checking it back on regenerates from the current
      name);
    - a **root** path text field and a tags/folders toggle;
    - a save button, plus error text and a busy state while `saving`.
- `Mode::Opening`: progress while `Manager::open` runs (same shell for
  first-boot create and later selects).
- Subsonic entries appear in Facet 3 (config variant + login mode).

**View stub:** `State::view` ships with a working `Select`/`Opening` rendering,
but the `AddOffline` arm returns a placeholder container with a
`// TODO(library_01)` comment pointing at the layout spec above — the form is
yours to implement.

### Wiring

- **`src/screen/mod.rs` (MainScreen):** `client: Arc<Client>` →
  `manager: Arc<Manager>`; `pub fn init(manager: Arc<Manager>) -> (Self,
  Task<Msg>)`; cover fetch in `apply_engine_event` via
  `manager.cover(track.cover…, COVER_PX)` (skipped when
  `track.cover.is_none()`); `Msg::CoverLoaded(CoverId, Result<Bytes, ()>)` keyed
  by cover id (stale-drop logic unchanged);
  `collect_shuffled_songs(manager, artist_id)` → `manager.artist(id)` → for each
  album `manager.album(id)` → flatten songs → `fisher_yates` (shuffle stays in
  the caller). Child screens get `manager` through `Ctx`.
- **`src/screen/artists.rs`:** `ArtistId3` → `library::Artist`; `State` holds
  only rows/loading state, not `Arc<Manager>`; covers via
  `ctx.manager.cover(artist.cover…, COVER_PX)` in `update`; empty state:
  distinguish "still loading" from "no artists found" (a loaded empty list shows
  the latter instead of the eternal `loading...` today).
- **`src/engine/mod.rs`:** `pub type Track = crate::library::Track;`;
  `EngineError::Subsonic` → `Library(#[from] LibraryError)`.
- **`src/engine/mock.rs`:** duration reads adapt to `Duration`
  (`track.duration.as_secs_f32()` instead of `Option<i64>` secs); title/ artist
  field types are unchanged (`String` / `Option<String>`).
- **`src/main.rs`:** `screen::login` → `screen::library_setup`;
  `Msg::LoggedIn(Client)` → `Msg::LibraryReady(Arc<Manager>)`.

### Facet 2 verification

- `cargo build` + `cargo clippy -- -D warnings` clean.
- Until the `AddOffline` view is implemented, exercise the flow with a
  hand-written `libraries.toml`: boot → select screen lists the entry → open →
  artists table populates → shuffle → mock now-playing bar shows
  title/artist/cover. Repeat with a folder-structured tree (`scan = "folders"`).
  Add a second offline library with `enabled = false` → greyed in select, absent
  from the artists table; flip it to `true` → both libraries' artists appear
  merged, ids prefixed with their identifiers.
- Once the form view exists: first boot (no `libraries.toml`) → form appears,
  identifier follows the name while auto is checked, typing an identifier
  unchecks auto, saving lands in the select list and opens the library.

## Facet 3 — subsonic provider (restores server support; no new deps)

`src/library/subsonic.rs`:

```rust
pub struct SubsonicLibrary {
    client: Arc<opensubsonic::Client>,
}

impl SubsonicLibrary {
    pub async fn login(url: &str, username: String, password: String)
        -> Result<Self, LibraryError>;   // Client::new + ping
}
```

- Same four methods as offline; converts `ArtistId3`/`AlbumWithSongsId3`/
  `Child` → neutral types (`cover_art` → `CoverId`, `duration: Option<i64>` secs
  → `Duration`), filters `!song.is_dir`, maps `opensubsonic::Error` →
  `LibraryError::Subsonic` (variant added).
- **Absent optional counts fall back to `0` with a `warn!`**: when
  `ArtistId3.album_count` or `AlbumId3.song_count` is `None`, use `0` and emit
  a `warn!` carrying the library identifier, the artist/album id + name, and
  which field was missing — the log is the only signal that the server
  didn't report the count. (The repo has no logging crate yet — logs are
  `eprintln!` — so either add a thin `warn!` macro over `eprintln!` or match
  the existing `[module]` prefix style; the doc's `warn!` names the intent.)
- `src/library/mod.rs`: add the `Library::Subsonic` variant + delegation arms;
  `LibraryConfig::Subsonic { name, id, url, username, password, enabled }`.
- `src/library/manager.rs`: no changes — the new variant is transparent to
  routing, merging, and the cache seam.
- `src/screen/library_setup.rs`: subsonic entries are listed (enabled ones
  clickable, like offline ones); clicking one pings via `SubsonicLibrary::login`
  (credentials come from the config — no re-typing). A `Mode::AddSubsonic` login
  form (url/username/password) is added next to `AddOffline` so servers can be
  configured without editing TOML.
- No other UI changes: artists table, shuffle, covers, and now-playing all
  already flow through the manager. An artists table mixing an offline library
  and a subsonic server is the merge working as designed.

### Facet 3 verification

- `cargo build` + `cargo clippy -- -D warnings` clean.
- With the server back: select → artists from both the offline library and the
  subsonic server appear merged; covers load; shuffle plays through the mock
  engine — behavior identical to today for the server, now through the seam.
  Kill the server mid-load → error surfaces on the select screen, app stays
  alive. Disable the server entry → only offline artists remain.

## Changes to existing files

| File                                      | Change                                                                                     |
| ----------------------------------------- | ------------------------------------------------------------------------------------------ |
| `Cargo.toml`                              | Facet 1: `lofty`                                                                           |
| `src/lib.rs`                              | `pub mod library;`                                                                         |
| `src/library/mod.rs`                      | types + `Library` enum + `LibraryError` + `LibraryConfig`/`ScanMode` + `slugify` (Facet 1) |
| `src/library/manager.rs`                  | `Manager` + `CacheStore` + prefixing/merge (Facet 1)                                       |
| `src/library/offline/{mod,scan,store}.rs` | offline provider (Facet 1)                                                                 |
| `src/library/subsonic.rs`                 | subsonic provider (Facet 3)                                                                |
| `src/config.rs`                           | `load_libraries`/`save_libraries` file IO; drop `Credentials` (Facet 2)                    |
| `src/screen/library_setup.rs`             | new screen, replaces `login.rs`; `AddOffline` view stub (Facet 2)                          |
| `src/screen/mod.rs`                       | owns `Arc<Manager>`, builds `Ctx`, cover via manager, shuffled-songs via manager (Facet 2)  |
| `src/screen/artists.rs`                   | neutral `Artist`, receives manager via `Ctx`, empty state (Facet 2)                       |
| `src/engine/mod.rs`                       | `Track` alias → `library::Track`; `EngineError` (Facet 2)                                  |
| `src/engine/mock.rs`                      | `Duration` reads (Facet 2)                                                                 |
| `src/main.rs`                             | `LibraryReady(Arc<Manager>)`, screen rename (Facet 2)                                      |
| `tests/fixtures/music/**`                 | fixture trees for scan + manager tests (Facet 1)                                           |

## Out of scope / future work

- `CacheStore::Sqlite` (rusqlite, `data_dir()/library.sqlite`) — metadata
  records for every provider in one place; cover and stream blob caching rides
  the same seam. Persisted ids need nothing from config: identifiers are already
  stable keys.
- Merge-policy change — per-library grouping/tabs, a source filter, or
  dedup-by-name across providers; lives entirely in `Manager::artists`.
- Enable/disable toggle + full library management (rename, edit, delete) in a
  settings screen, including editing an existing library's identifier;
  continuing past a failed library entry with a warning instead of hard error.
- Rescan/refresh (re-run a provider's scan, then invalidate the sqlite cache);
  `Manager::stream(&TrackId)` for real engine playback (engine Facet 2) and
  `Manager::scrobble` (engine Facet 4); `Scrobble`-capable provider variants.
- Cover resizing in the offline store; smb/youtube providers; migrating any
  `credentials.toml` found on disk (subsonic is dead weight until the server
  returns — re-entering creds in Facet 3 is one form).

## Verification (whole plan)

Per-facet checks above; `cargo build` / `cargo clippy -- -D warnings` after
every facet, `cargo test` from Facet 1 on; final manual run proving the app
fully works offline once the `AddOffline` view is implemented (Facet 2), then
the subsonic regression check once the server is back (Facet 3).
