# Library database 01 — Tokio, per-library metadata DBs, global DB, and scans

Applies to: `Cargo.toml`, `src/main.rs`, `src/screen/mod.rs`,
`src/screen/*`, `src/screen/library_setup/*`, `src/library/*`,
`src/engine/real.rs`, `migrations/`, and new `src/library/db/*` /
`src/screen/covers.rs` modules.

## 1. Goal

Stop re-walking and re-tagging every library at startup. Each library gets its
own SQLite metadata database, created on first open and updated by background
scans. Metadata queries go through the manager, which routes to the owning
library, which queries its own database file.

At the same time:

- switch the async runtime from smol to tokio;
- add a separate global database for playlists and play history;
- remove the in-memory metadata store entirely;
- make background rescans safe while the UI reads;
- notify screens when a library changes so they reload and can show a
  per-screen Not Found state;
- keep caching per library, with blobs on disk and no blobs in SQLite.

## 2. Locked in decisions

These are settled from the design discussion. Later sections explain each.

### Runtime and storage

- **Runtime: tokio.** `iced` switches from the `smol` feature to `tokio`.
  SQLx uses `runtime-tokio`.
- **One metadata DB per library**, plus **one global DB** for durable app data.
- **DBs live under `data_dir()`**: `~/.local/share/coldsonic` on Linux.
- **No metadata in memory.** `MemoryStore` is deleted. SQLite is the index.
  Cover/audio bytes are returned directly to the caller, never parked in a
  manager-owned map.
- **Relative paths** are stored in the DB. Network shares are unsupported.
  Symlinks are skipped in v1.
- **Blobs never go in SQLite.** Extracted embedded art goes to the per-library
  cache directory; folder art is read from the library root.

### Query and ownership model

- **The manager is the only seam.** Screens never see a provider.
- **`MainScreen` owns `Manager` by value.** There is no `Arc<Manager>`.
- **Async work gets a `ManagerHandle`**, a cloneable handle over an immutable
  catalog of DB pools, roots, and the event sender. The handle can read
  metadata but cannot mutate app state.
- **Keep the `Library` provider enum.** `OfflineLibrary` owns the DB; future
  online providers reuse the same manager routing and the same schemas.
- **Unprefixed ids inside a library DB.** The manager prefixes on the way out
  and strips on the way in. The global DB stores prefixed ids.

### Scan behavior

- **Full scan only when the DB has never completed a scan, when the scan
  signature changes, or when explicitly triggered.**
- **Startup runs a stat-walk check** comparing `(path, mtime_ns, size, kind)`
  against the `files` table. Only a difference triggers a full scan.
- **Watcher changes trigger a debounced full scan.** Incremental scanning is a
  future optimization; the `files` table is recorded now to enable it.
- **A scan runs in an iced task and returns a status message.** The task writes
  the DB; `MainScreen` applies the result through the manager, which bumps the
  revision and emits an event.
- **Metadata writes happen in one short transaction**, after the walk and tag
  reads are complete. No transaction spans the filesystem walk.

### UI behavior

- **Per-screen load state**: `Loading | Loaded | NotFound | Error`.
- **Not Found is a state, not a route.**
- **Library events mark every navigation entry dirty.** The top entry reloads
  immediately; deeper entries reload when they become top.
- **Cover fetches are deduplicated by `CoverId` in the screen layer.**
- **Global DB owns playlists and play history.** Schema and manager methods land
  now; playlist UI is out of scope.

## 3. Current path vs. target path

### Today

```text
make_screen -> album_detail::init(id, Arc<Manager>)
Task::perform(manager.album(&id))
  Manager::album
    -> route("alpha:...") -> Opened
    -> Library::Offline -> OfflineLibrary::album
    -> MemoryStore hashmap lookup (in-memory index built at open)
  re-prefix ids
Msg::AlbumDetailMsg(Msg::Loaded(...))
  -> MainScreen::update
  -> album_detail::update stores the owned AlbumDetail
  -> optional cover task -> Manager::cover -> MemoryStore cover map -> Bytes
```

`MemoryStore` is built by a full walk + tag pass in `OfflineLibrary::open`.
Covers are held as `Vec<u8>` in RAM for the whole session.

### Target

```text
make_screen -> album_detail::init(id, self.manager.handle())
Task::perform(handle.album(&id))
  ManagerHandle routes "alpha:..." -> LibraryHandle { root, pool }
  Library::Offline -> OfflineLibrary::album
    -> LibraryDb (SqlitePool) query
  rows -> AlbumDetail -> re-prefix ids
Msg::AlbumDetailMsg(Msg::Loaded(...))
  -> MainScreen::update (Ctx.manager: &Manager)
  -> album_detail::update stores the owned AlbumDetail
  -> optional cover task -> ManagerHandle::cover -> path -> file read -> Bytes
```

The only in-memory state is the DB pool, the roots, and the UI's own state.

## 4. Ownership, manager, and handles

```rust
pub struct Manager {
    catalog: Arc<Catalog>,                  // immutable after open
    events: broadcast::Sender<ManagerEvent>,
    revision: u64,                          // bumped on every library update
    scan_state: HashMap<String, ScanState>, // idle/scanning/dirty
}

struct Catalog {
    libraries: Vec<Opened>,
    global: GlobalDb,
}

struct Opened {
    config: LibraryConfig,
    library: Library, // unchanged provider seam
}

#[derive(Clone)]
pub struct ManagerHandle(Arc<Catalog>);

#[derive(Clone)]
pub enum LibraryHandle {
    Offline(OfflineHandle),
}

#[derive(Clone)]
pub struct OfflineHandle {
    identifier: String,
    root: PathBuf,
    pool: SqlitePool,
}
```

Rules:

- `Manager::handle()` clones one `Arc`. That is the only refcount bump per
  task; it does not clone catalog contents.
- Query methods live on a shared surface used by both `Manager` and
  `ManagerHandle` (delegating to `Catalog`) so they cannot drift:
  `artists`, `albums`, `tracks`, `artist`, `album`, `cover`, `track_bytes`.
- A handle is a **read-only routing snapshot**. It cannot mutate libraries,
  reach `MainScreen`, reach the engine, or hold cover bytes.
- `Ctx` changes from `manager: &'a Arc<Manager>` to `manager: &'a Manager`.
  `Ctx` remains `Copy`.
- `Manager` still exposes the query methods so tests and non-task code can call
  them directly.
- `RealEngine` holds a `ManagerHandle` for `track_bytes`.
- `library_setup` emits an owned `Manager`; `main.rs` uses
  `Msg::LibraryReady(Manager)`.
- Library add/remove is not supported in this plan. Handles are valid for the
  lifetime of the process; if add/remove is added later, handles must be
  recreated after a catalog change.

### Why not borrow the manager in a task?

`Task::perform` needs a `'static` future. A `&Manager` borrowed from
`MainScreen::update` cannot cross into it. The handle is the smallest thing
that satisfies `'static` without exposing the whole manager. The engine has the
same constraint and uses the same handle.

### Why this is not "Arc<Manager>" again

`Arc<Catalog>` is internal to `Manager` and `ManagerHandle`. `Catalog` is
immutable after open, contains no `MainScreen`/`Engine`/UI state, and exposes
only read operations. The old `Arc<Manager>` let async code reach the entire
manager by reference; the new handle cannot.

## 5. Tokio migration

Do this first, as an isolated refactor, keeping the current in-memory behavior.

### Cargo

```toml
iced = { version = "0.14", features = ["tokio", "image", "advanced"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "fs", "sync"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "migrate"] }
notify = "8"
notify-debouncer-full = "0.5"
futures = "0.3"
```

- Remove `smol`.
- Remove `async-compat` if nothing needs it after the SQLite work.
- `iced`'s tokio backend creates and owns a multi-thread runtime, so pool
  creation, watcher tasks, and scans all share it.

### Call-site changes

| smol | tokio |
| --- | --- |
| `smol::unblock(f)` | `tokio::task::spawn_blocking(f).await?` |
| `smol::fs::read(p)` | `tokio::fs::read(p).await` |
| `smol::Timer::after(d)` | `tokio::time::sleep(d).await` |
| `smol::stream::Stream` | `futures::stream::Stream` |
| `smol::block_on(f)` in tests | `#[tokio::test]` or a `block_on` test helper |

`src/screen/debug.rs` and `src/screen/mod.rs` use `smol::Timer`; the offline
provider uses `smol::unblock` / `smol::fs`. Tests use `smol::block_on`
throughout `src/library/*`.

### Runtime-context rule

Create pools, watchers, and workers from inside an iced `Task` or subscription,
never from synchronous `init` code. `Manager::open` is already awaited inside
`library_setup`'s task, so it has runtime context.

## 6. Database layout and settings

### Paths

```text
~/.local/share/coldsonic/            (data_dir())
├── global.sqlite
└── libraries/
    ├── {identifier}.sqlite
    └── {identifier}.sqlite-wal / -shm

~/.cache/coldsonic/                  (cache_dir())
└── libraries/
    └── {identifier}/
        └── covers/                  extracted embedded art only
```

`data_dir()` and `cache_dir()` already exist in `src/config.rs`. Identifiers
are validated `[a-z0-9-]+`, so they are safe filenames.

### Pool settings

Apply to every SQLite pool:

- `journal_mode = WAL`
- `foreign_keys = ON`
- `busy_timeout = 5000`
- `synchronous = NORMAL`
- `create_if_missing(true)` for library DBs
- small pool (SQLite has one writer; 2–4 connections is enough)

### Testing paths

`Manager::open` uses `config::data_dir()` / `config::cache_dir()`. Add a
test-only constructor that takes explicit directories so tests never write to
the real user data directory:

```rust
pub async fn open(configs: &[LibraryConfig]) -> Result<Manager, LibraryError>;
pub async fn open_at(
    configs: &[LibraryConfig],
    data_dir: &Path,
    cache_dir: &Path,
) -> Result<Manager, LibraryError>;
```

## 7. Library DB schema

IDs are unprefixed and provider-local. `files` is separate so normal queries
never touch `mtime`/`size`.

```sql
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- schema_version, scanner_version, scan_mode, never_split_hash, root,
-- identifier, scan_complete, last_scan_unix_nanos

CREATE TABLE artists (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    sort_name   TEXT,
    album_count INTEGER NOT NULL DEFAULT 0,
    cover_id    TEXT
);

CREATE TABLE covers (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,          -- 'folder' | 'embedded'
    source_path TEXT NOT NULL,          -- relative to library root
    cache_path  TEXT,                   -- relative to library cache dir, embedded only
    mime        TEXT,
    mtime_ns    INTEGER,
    size        INTEGER
);

CREATE TABLE albums (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    artist_id   TEXT REFERENCES artists(id) ON DELETE SET NULL,
    artist_name TEXT,
    song_count  INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER,
    cover_id    TEXT REFERENCES covers(id) ON DELETE SET NULL
);

CREATE TABLE tracks (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    artist_id   TEXT REFERENCES artists(id) ON DELETE SET NULL,
    artist_name TEXT,
    album_id    TEXT REFERENCES albums(id) ON DELETE SET NULL,
    album_name  TEXT,
    track_no    INTEGER,
    duration_ms INTEGER NOT NULL,
    cover_id    TEXT REFERENCES covers(id) ON DELETE SET NULL
);

CREATE TABLE files (
    id       INTEGER PRIMARY KEY,
    path     TEXT NOT NULL UNIQUE, -- relative to library root, '/' separators
    mtime_ns INTEGER NOT NULL,
    size     INTEGER NOT NULL,
    kind     TEXT NOT NULL,        -- 'audio' | 'cover'
    track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL
);

CREATE INDEX albums_name_idx   ON albums(name);
CREATE INDEX tracks_title_idx  ON tracks(title);
CREATE INDEX tracks_album_idx  ON tracks(album_id);
CREATE INDEX tracks_artist_idx ON tracks(artist_id);
CREATE INDEX files_track_idx   ON files(track_id);
```

Notes:

- No circular FK. Only `files.track_id`; there is no `tracks.file_id`.
- `covers` is created before `albums`/`tracks` so the references read cleanly.
- Offline ids keep their current shapes: `ArtistId` = artist name,
  `AlbumId` = `artist\u{0}album`, `TrackId` = relative path, `CoverId` = album
  directory (folder art) or audio file path (embedded art).
- Audio whitelist (shared with the scanner):
  `mp3`, `flac`, `ogg`, `opus`, `m4a`, `wav`.
- Folder-art whitelist (case-insensitive):
  `cover.jpg`, `cover.png`, `folder.jpg`, `folder.png`.
- Paths stored relative to the library root with `/` separators.
- Symlinks are skipped in v1. Loop detection is unnecessary because symlinks
  are not followed.

### Schema lifecycle

Library metadata is derived data, so no migrations are needed after v1:

- `meta.schema_version` must match `SCHEMA_VERSION`; if not, drop the tables,
  recreate the schema, and set `scan_complete = 0`.
- `meta.scan_mode`, `meta.never_split_hash`, `meta.root`, and
  `meta.scanner_version` form the scan signature; a mismatch sets
  `scan_complete = 0`.
- `scan_complete = 1` plus a clean stat-walk means the DB is usable as-is.

## 8. Global DB schema

Durable user data, migrated with sqlx. Track references use **prefixed** ids.

```sql
CREATE TABLE playlists (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE playlist_tracks (
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    track_id    TEXT NOT NULL,  -- prefixed
    library_id  TEXT NOT NULL,
    title       TEXT,
    artist      TEXT,
    album       TEXT,
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
);

CREATE TABLE play_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id    TEXT NOT NULL,  -- prefixed
    library_id  TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    played_ms   INTEGER,
    completed   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX play_history_started_idx ON play_history(started_at);
```

- Snapshot fields let a playlist render an entry whose library/track is gone.
- Cross-DB operations are not transactional. A library can disappear while a
  playlist references it; resolve at read time and render unavailable entries
  with the Not Found treatment.
- Manager methods land now: playlist CRUD, add/remove/reorder tracks, and
  `record_play`. No playlist UI in this plan.

## 9. Opening libraries

`Manager::open` is fast and does no scanning:

1. Open and migrate `global.sqlite`.
2. Validate identifiers (existing behavior).
3. For each enabled config:
   - create `data_dir()/libraries/` if needed;
   - open `{identifier}.sqlite` with `create_if_missing(true)`;
   - apply pragmas;
   - create/migrate the schema;
   - check the scan signature and `scan_complete`.
4. Build `Catalog` and return `Manager`.

Because the pool is opened here, **every library always has a pool**. A library
that has never been scanned has a valid empty schema and `scan_complete = 0`,
not a missing file.

### Deviation from the earlier draft: no temp-DB rename

The earlier draft created the initial DB as `{id}.sqlite.tmp` and renamed it
into place. That was safe only while no pool existed. With a handle-based
catalog, the pool must exist before any screen or the engine can query, and
replacing the file under a live pool is unsafe. Eager creation removes the only
safe window for the rename.

The short-transaction rule already covers the initial scan:

- walk and read tags into memory (no DB lock);
- open one `BEGIN IMMEDIATE` transaction;
- delete and insert rows, set `scan_complete = 1`;
- commit.

That is the same path for first scan and rescans, with no pool swap and no
handle invalidation. If a future need makes temp files attractive again, the
alternative is a per-library `ArcSwap<LibraryDb>` inside the catalog; that is
more machinery than this plan needs.

## 10. Scan pipeline

New module `src/library/scan.rs` (orchestration) plus DB writers in
`src/library/db/`.

### 10.1 Scan data

`scan::walk_and_tag(root, mode, never_split)` runs inside
`spawn_blocking` and produces:

```rust
pub struct ScanData {
    pub artists: Vec<ScanArtist>,
    pub albums: Vec<ScanAlbum>,
    pub tracks: Vec<ScanTrack>,
    pub covers: Vec<ScanCover>,
    pub files: Vec<ScanFile>,
}

pub struct ScanCover {
    pub id: String,
    pub kind: CoverKind,        // Folder | Embedded
    pub source_path: String,    // relative
    pub mime: Option<String>,
    pub mtime_ns: i64,
    pub size: i64,
    pub bytes: Option<Vec<u8>>, // Some only for embedded art
}
```

- One lofty read per audio file, as today.
- Metadata only; no cover bytes are retained after the DB write.
- Embedded art is the only blob present during the scan and is written to the
  cache directory before the scan data is dropped.
- Empty library is not an error. It completes the scan with zero rows.

### 10.2 DB write

After the walk/tags finish:

1. For each embedded cover, write bytes to
   `cache_dir()/libraries/{id}/covers/{stable_name}` (idempotent; the filename
   is derived from the cover id). Record the relative cache path.
2. Open a single write transaction on the pool.
3. `DELETE FROM` tracks, albums, artists, covers, files (child order).
4. Batch-insert artists, covers, albums, tracks, files.
5. Update `meta` (`scan_complete = 1`, `last_scan_unix_nanos`, signature).
6. Commit.
7. Drop `ScanData` (frees embedded bytes).

The transaction covers only steps 3–6. Readers in WAL mode continue to see the
previous snapshot until commit.

### 10.3 Task and result

`MainScreen` spawns one scan task per library:

```rust
Task::perform(
    async move { scan::library(handle, config).await },
    |outcome| Msg::LibraryScanFinished(outcome),
)
```

```rust
pub struct ScanOutcome {
    pub library: String,
    pub result: Result<(), String>,
}
```

The task does not return metadata in the message. The DB is the handoff.

`MainScreen` then calls the manager (the only place manager state changes):

```rust
match self.manager.finish_scan(&outcome.library, outcome.result) {
    // bumps revision + emits LibraryUpdated, or emits LibraryScanFailed
}
```

`begin_scan` returns `false` if a scan is already running and sets the library
dirty instead. `finish_scan` returns whether a follow-up scan is needed because
the library became dirty during the scan.

### 10.4 Per-library scan state

```text
Idle -> Scanning -> Idle
          ^  |
          |  v
        Dirty (change arrived while scanning)
```

- At most one scan per library.
- Events during `Scanning` set `Dirty`.
- When the scan finishes and `Dirty` is set, run the stat-walk again; scan if
  needed.
- Different libraries may scan in parallel.

## 11. Startup checks

Startup does **not** scan. `MainScreen::init` (or the first subscription tick)
spawns one check task per library:

1. If `meta.scan_complete != 1` or the scan signature differs → full scan.
2. Otherwise run `scan::check(root, pool)` in `spawn_blocking`:
   - walk the root with the whitelist, skipping symlinks;
   - load `files` into a map;
   - diff by `(path, mtime_ns, size, kind)`;
   - report `Clean` or `NeedsScan(reason)`.
3. `Clean` → nothing. `NeedsScan` → full scan.

The stat-walk is cheap relative to tag reads and catches old-mtime copies and
clock changes that a `last_scan` comparison would miss. `last_scan_unix_nanos`
is stored for diagnostics only.

Until the check finishes, screens query the existing DB and show its contents.
A first run shows the normal loading/empty state until the scan commits.

## 12. File watcher

- Use `notify` + `notify-debouncer-full` (or the equivalent debouncer API).
- Watch only the library root, recursively. DB and cache directories are
  outside it, so the app never reacts to its own writes.
- Implement the watcher as a `Subscription` per library. The subscription owns
  the debouncer and emits `Msg::LibraryChanged(library_id)` after a quiet
  period (~1 s).
- `Msg::LibraryChanged` → `manager.mark_dirty(id)` and, if not scanning, run
  the same stat-walk/scan check as startup.
- Start watchers when `MainScreen` subscribes (libraries already exist). There
  is no self-write storm to avoid because the root is never written.
- On watcher failure (inotify limits, permissions, unsupported filesystem):
  log once, disable that library's watcher, and rely on startup checks plus
  manual rescan. Do not crash the app.

A manual rescan action in library setup triggers the same check/scan path; it
is the recovery path for a disabled watcher and for missed changes.

## 13. Manager events and screen refresh

```rust
#[derive(Debug, Clone)]
pub enum ManagerEvent {
    LibraryScanStarted { library: String },
    LibraryUpdated { library: String, revision: u64 },
    LibraryScanFailed { library: String, error: String },
}
```

- `Manager` owns `broadcast::Sender<ManagerEvent>`; `ManagerHandle` also holds
  a clone so future workers can emit.
- `MainScreen::subscription` subscribes, maps to `Msg::Manager(event)`, and
  handles `Lagged` by forcing a full reload of the top screen.
- Ordering is strict: **commit → bump revision → emit**. Reloads triggered by
  an event must see the new data.

### Stale results

- `Manager` keeps one global monotonic `revision`, bumped on every successful
  library update. The revision is included in `LibraryUpdated`.
- Screens store `revision: u64` (initialized from `manager.revision()` at
  creation).
- A fetch captures the current revision at task creation and carries it back.
  A result is applied only if its revision matches the screen's current one.
- Any manager event invalidates in-flight fetches: set `revision`, mark dirty,
  reload.
- A global revision may invalidate a fetch for a different library; that is
  harmless (one extra fetch).

### Refresh protocol

Each screen:

- stores `dirty: bool` and `revision: u64`;
- handles `Msg::Reload` by starting a fresh fetch (preserving selection and
  scroll where possible);
- uses `LoadState<T>`:

```rust
pub enum LoadState<T> {
    Loading,
    Loaded(T),
    NotFound,
    Error(String),
}
```

On `LibraryUpdated`:

1. `MainScreen` sets the revision from the event and marks **every** stack
   entry dirty.
2. If the top entry is dirty, it sends its `Reload` message immediately.
3. When navigating (push/back), a dirty entry reloads before it is shown.
4. Screens keep their route and stack position; `NotFound` is rendered in
   place, distinct from a legitimate empty list.

`LibraryScanStarted` can drive a non-intrusive "refreshing" indicator; it is
not required for correctness. `LibraryScanFailed` logs and leaves the old data
visible.

### Covers on refresh

- On `LibraryUpdated`, clear the screen's `CoverMap` before reloading rows.
- Refetch only covers still referenced after the reload.
- Key cover results by `CoverId`; ignore results for ids no longer present.

## 14. Covers

No cover bytes live in the manager or in a metadata store.

- Folder art: `covers.source_path` is opened directly from the library root.
- Embedded art: extracted during the scan, written to
  `cache_dir()/libraries/{id}/covers/`, and recorded as `cache_path`. The
  bytes are read on demand.
- `Library::cover(id, px)` returns `Bytes` to the caller. Offline providers do
  not resize; `px` stays in the signature for online providers.

### Screen dedupe

All list screens use one shared helper (new `src/screen/covers.rs`), modeled on
the existing `all_tracks` implementation:

```rust
pub struct CoverMap {
    states: HashMap<CoverId, CoverState>,
}

impl CoverMap {
    /// Drop entries no longer wanted, start one fetch per missing cover id,
    /// return the tasks. Existing Loading/Loaded entries are left alone.
    pub fn sync(
        &mut self,
        wanted: impl IntoIterator<Item = CoverId>,
        px: u32,
        handle: &ManagerHandle,
    ) -> Task<(CoverId, Result<Bytes, ()>)>;
}
```

- `all_tracks` already deduplicates correctly; extract the pattern.
- `albums` and `artists` currently spawn one task per row and must switch to
  the helper.
- Deduplication is by `CoverId`, not "all rows have the same cover": distinct
  covers still fetch once each.
- `AlbumDetail` needs a single cover; no map required there.

## 15. Caching

- Cache is per library.
- **No blobs in SQLite.**
- Blob location: `cache_dir()/libraries/{id}/...`; metadata index may live in
  the library DB.
- Offline audio and folder art are read from the library root, not copied into
  the cache. Only extracted embedded art is cached.
- Deleting `cache_dir()` costs a rescan at worst; no user data is lost.
- Online libraries (future) reuse the same per-library layout, with cache
  index tables added to their library DB. Do not mix enrichment data with
  scanner metadata.

## 16. Consistency rules

- **No metadata swap.** The DB is the source of truth; WAL gives readers a
  consistent pre-commit snapshot.
- **Short write transactions.** Build all scan data first, then write.
- **Read transactions for multi-statement queries.** `album()` reads album +
  tracks; `artist()` reads artist + albums. Wrap those in one `BEGIN DEFERRED`
  read so a rescan commit cannot produce a mixed result.
- **Single-statement queries** need no transaction.
- **Eager pool creation** means no pool replacement and no stale handle.
- **Event order**: DB commit → revision bump → event.
- **Scans are serialized per library**; different libraries may scan in
  parallel.
- **Shutdown**: dropping pools closes connections and rolls back uncommitted
  work. `spawn_blocking` work cannot be cancelled; accept that shutdown may
  wait for the current walk, or chunk it later.
- **Global DB is separate**, so playlist/history writes never contend with
  library scans; cross-DB consistency is resolved at read time.

## 17. Implementation phases

Keep the tree compiling and the app runnable after each phase.

1. **Tokio migration**
   - Cargo features; replace smol call sites; migrate tests.
   - Verify playback, scan-at-open, timers, `dev` build.
2. **Manager ownership refactor**
   - `MainScreen` owns `Manager`; add `Arc<Catalog>` + `ManagerHandle`.
   - Remove `Arc<Manager>` from screens, engine, library setup, `main.rs`.
   - `Ctx.manager: &Manager`.
   - Keep the in-memory provider for now; behavior unchanged.
3. **SQLx foundation**
   - Add SQLx and the two migration sets.
   - Add `open_at` and `GlobalDb` / `LibraryDb` open + pragma + migrate
     helpers.
   - Open global + library DBs at startup; no scans yet; screens still use the
     in-memory store.
   - Set up `.sqlx` prepare with a combined dev DB (see §18).
4. **Library schema + writer**
   - Implement the schema, `ScanData -> DB` writer, embedded-art cache write.
   - Rewrite `OfflineLibrary` to query SQL:
     `artists`, `albums`, `tracks`, `artist`, `album`, `cover`, `track_bytes`.
   - Delete `MemoryStore` and the old open-time scan.
   - `track_bytes` resolves the relative path from the track id and reads the
     file (no DB query needed).
5. **Scan tasks + startup checks**
   - `scan::walk_and_tag`, `scan::check`, `scan::library`.
   - `Msg::LibraryScanFinished`, manager `begin_scan`/`finish_scan`,
     revision, `ManagerEvent`.
   - Startup stat-walk; missing/incomplete DB triggers a scan.
   - Manual rescan action in library setup.
6. **Screen refresh + Not Found**
   - `Manager` subscription in `MainScreen`.
   - `LoadState`, `dirty`, `revision`, `Reload` in every screen.
   - Shared `CoverMap`; migrate `albums` and `artists`; keep `all_tracks`
     behavior.
   - Not Found rendering for detail screens.
7. **File watcher**
   - `notify` debouncer subscription; dirty marking; check/scan trigger.
   - Failure fallback and manual rescan path.
8. **Global DB data**
   - Playlist and play-history manager methods.
   - No playlist UI.

Update `change_docs/library_01.md` (and `change_docs/engine_01.md` if the
engine handle changes its wording) after the behavior lands.

## 18. Verification

### Commands

```sh
cargo fmt
cargo check
cargo check --features dev
cargo test
cargo clippy --all-targets --features dev -- -D warnings
SQLX_OFFLINE=true cargo check
```

### SQLx offline mode

- Use `query!` / `query_as!` for static queries; `query` / `QueryBuilder` for
  dynamic filters.
- `cargo sqlx prepare` needs one `DATABASE_URL`. Global and library tables are
  disjoint, so keep a combined dev database (`dev/offline.sqlite`) that has
  **both** schemas applied. Prepare against it and commit `.sqlx/`.
- Build with `SQLX_OFFLINE=true` in CI.
- Integration tests create their own temp DBs by running the migrations, not
  by sharing the dev database.

### Tests

- schema creation and `user_version`/meta checks;
- first scan → DB rows match the fixture tree;
- reopening skips the scan and returns identical artists/albums/tracks;
- stat-walk detects added, removed, and modified files;
- rescan replaces rows atomically, leaves no orphans;
- reader sees the old snapshot while a rescan transaction is open;
- relative paths, whitelist, and symlink skipping;
- empty library is not an error;
- embedded cover is cached on disk and `cache_path` resolves;
- folder art is read from the library root;
- cover dedupe fetches one task per distinct `CoverId`;
- revision guard ignores stale reload results;
- Not Found when a detail id disappears;
- global playlist/history CRUD and dangling-track reads;
- `Manager::open_at` never touches the real data/cache dirs.

### Manual

- First run with no DB: UI opens immediately, scan runs in background,
  metadata appears after commit.
- Second run: metadata is instant; stat-walk is clean; no scan.
- Change a file while running: watcher triggers a debounced scan; screens
  refresh.
- Delete the open artist/album externally: after refresh, the screen shows
  Not Found and Back still works.
- Add a track externally: it appears after the scan.
- Replace a cover file: cover updates after refresh.
- Playback, settings, panels, and queue behavior are unchanged.
- `dev` build still works.

## 19. Risks and open items

- **SQLx + tokio + iced**: validate pool creation inside iced tasks, offline
  prepare with two schemas, and `spawn_blocking` behavior before building the
  full scan pipeline. This is the highest-risk integration point.
- **Full rescan on every watcher burst** is simple but can be expensive on a
  large library. Incremental updates using `files` are the follow-up.
- **Embedded cover extraction** keeps blobs out of the DB but adds cache
  writes during scans. If cache writes fail (read-only cache dir), the scan
  should still commit and mark those covers unavailable, not fail the whole
  library.
- **Watcher limits/unsupported filesystems**: fallback is startup checks plus
  manual rescan.
- **Global dangling references**: playlists and history must render
  unavailable entries gracefully.
- **Library add/remove** is not in scope; handles are process-lifetime
  snapshots.
- **Playlist UI** is not in scope; only schema and manager methods.
- **Network shares** are explicitly unsupported.
