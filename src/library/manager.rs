//! Manager — the only library seam the UI touches.
//!
//! `MainScreen` owns a [`Manager`] by value. Async work (scans, the engine,
//! screen fetches) gets a [`ManagerHandle`]: one `Arc` over an immutable
//! catalog of pools, roots, and the event sender. A handle can read metadata
//! but cannot mutate app state.
//!
//! The manager opens every enabled library's database eagerly. Opening is
//! fast and performs no scanning; startup checks and scans run later as iced
//! tasks. Ids are unprefixed inside a library database and prefixed on the way
//! out, so routing a query back down is pure string work.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::config;
use crate::library::db::global::GlobalDb;
use crate::library::db::library::{LibraryDb, ScanSignature};
use crate::library::db::{global_db_path, library_cover_cache_dir, library_db_path};
use crate::library::is_valid_id;
use crate::library::offline::{OfflineHandle, OfflineLibrary};

use super::{
    Album, AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, CoverId, Library, LibraryConfig,
    LibraryError, PlayHistoryEntry, Playlist, PlaylistTrack, Track, TrackId,
};

#[derive(Debug)]
pub struct Manager {
    catalog: Arc<Catalog>,
    /// Monotonic, bumped on every successful library update.
    revision: u64,
    scan_state: HashMap<String, ScanState>,
}

#[derive(Debug)]
struct Catalog {
    libraries: Vec<Opened>,
    global: GlobalDb,
    events: broadcast::Sender<ManagerEvent>,
}

#[derive(Debug)]
struct Opened {
    config: LibraryConfig,
    library: Library,
}

/// Read-only routing snapshot handed to tasks and the engine.
#[derive(Debug, Clone)]
pub struct ManagerHandle(Arc<Catalog>);

#[derive(Debug, Clone)]
pub enum LibraryHandle {
    Offline(OfflineHandle),
}

#[derive(Debug, Clone)]
pub struct ScanPlan {
    pub library: String,
    pub config: LibraryConfig,
    pub handle: LibraryHandle,
}

#[derive(Debug, Clone)]
pub struct ScanOutcome {
    pub library: String,
    pub result: Result<(), String>,
}

/// The manager's only outward-facing event surface.
#[derive(Debug, Clone)]
pub enum ManagerEvent {
    LibraryScanStarted {
        library: String,
    },
    LibraryUpdated {
        library: String,
        revision: u64,
    },
    LibraryScanFailed {
        library: String,
        error: String,
    },
    /// The subscriber fell behind the broadcast ring. The UI should ignore
    /// the missed details and reload the visible screen.
    Lagged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Idle,
    Scanning,
    Dirty,
}

impl Manager {
    /// Open every enabled library using the app's real data/cache dirs.
    pub async fn open(library_configs: &[LibraryConfig]) -> Result<Manager, LibraryError> {
        let data_dir = config::data_dir()
            .ok_or_else(|| LibraryError::Invalid("no data directory on this system".into()))?;
        let cache_dir = config::cache_dir()
            .ok_or_else(|| LibraryError::Invalid("no cache directory on this system".into()))?;
        Self::open_at(library_configs, &data_dir, &cache_dir).await
    }

    /// Test-only constructor: never touches the real user data/cache dirs.
    pub async fn open_at(
        library_configs: &[LibraryConfig],
        data_dir: &Path,
        cache_dir: &Path,
    ) -> Result<Manager, LibraryError> {
        let mut seen = HashSet::new();
        for config in library_configs {
            let id = config.identifier();
            if !is_valid_id(id) {
                return Err(LibraryError::Invalid(format!(
                    "invalid library identifier '{id}': expected [a-z0-9-]+"
                )));
            }
            if !seen.insert(id.to_string()) {
                return Err(LibraryError::Invalid(format!(
                    "duplicate library identifier '{id}'"
                )));
            }
        }

        let global = GlobalDb::open(&global_db_path(data_dir)).await?;

        let mut libraries = Vec::new();
        for config in library_configs {
            let LibraryConfig::Offline {
                name,
                id,
                root,
                scan,
                enabled,
                never_split,
            } = config;
            if !*enabled {
                tracing::debug!(%id, "skipping disabled library");
                continue;
            }
            tracing::info!(%id, %name, root = %root.display(), ?scan, "opening library");
            let db_path = library_db_path(data_dir, id);
            let db = LibraryDb::open(&db_path).await.map_err(|err| {
                LibraryError::Invalid(format!(
                    "failed to open library '{name}' (id '{id}'): {err}"
                ))
            })?;
            db.apply_signature(&ScanSignature::from_config(config))
                .await?;
            let cache_dir = library_cover_cache_dir(cache_dir, id);
            let offline = OfflineLibrary::new(
                OfflineHandle {
                    identifier: id.clone(),
                    root: root.clone(),
                    pool: db.pool.clone(),
                    cache_dir,
                },
                *scan,
                never_split.clone(),
            );
            libraries.push(Opened {
                config: config.clone(),
                library: Library::Offline(offline),
            });
        }

        let (events, _) = broadcast::channel(64);
        Ok(Manager {
            catalog: Arc::new(Catalog {
                libraries,
                global,
                events,
            }),
            revision: 0,
            scan_state: HashMap::new(),
        })
    }

    /// Clone one `Arc`. This is the only refcount bump a task needs.
    pub fn handle(&self) -> ManagerHandle {
        ManagerHandle(Arc::clone(&self.catalog))
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ManagerEvent> {
        self.catalog.events.subscribe()
    }

    /// One plan per opened library, in config order.
    pub fn scan_plan(&self) -> Vec<ScanPlan> {
        self.catalog.scan_plan()
    }

    pub fn begin_scan(&mut self, library: &str) -> bool {
        match self.scan_state.get(library).copied() {
            Some(ScanState::Scanning) => {
                self.scan_state
                    .insert(library.to_string(), ScanState::Dirty);
                false
            }
            _ => {
                self.scan_state
                    .insert(library.to_string(), ScanState::Scanning);
                let _ = self.catalog.events.send(ManagerEvent::LibraryScanStarted {
                    library: library.to_string(),
                });
                true
            }
        }
    }

    /// Apply a finished scan. Returns `true` when a follow-up scan is needed
    /// because a change arrived while the scan was running.
    pub fn finish_scan(&mut self, library: &str, result: Result<(), String>) -> bool {
        let was_dirty = self
            .scan_state
            .get(library)
            .is_some_and(|state| *state == ScanState::Dirty);
        self.scan_state.insert(library.to_string(), ScanState::Idle);

        match result {
            Ok(()) => {
                self.revision += 1;
                let _ = self.catalog.events.send(ManagerEvent::LibraryUpdated {
                    library: library.to_string(),
                    revision: self.revision,
                });
                was_dirty
            }
            Err(error) => {
                tracing::warn!(%library, %error, "library scan failed");
                let _ = self.catalog.events.send(ManagerEvent::LibraryScanFailed {
                    library: library.to_string(),
                    error,
                });
                false
            }
        }
    }

    /// Mark a library dirty. Returns `true` when a scan should be started now.
    pub fn mark_dirty(&mut self, library: &str) -> bool {
        match self.scan_state.get(library).copied() {
            Some(ScanState::Scanning) => {
                self.scan_state
                    .insert(library.to_string(), ScanState::Dirty);
                false
            }
            _ => {
                self.scan_state
                    .insert(library.to_string(), ScanState::Dirty);
                true
            }
        }
    }

    // --- Query surface (delegates to the catalog) -------------------------

    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        self.catalog.artists().await
    }

    pub async fn albums(&self) -> Result<Vec<Album>, LibraryError> {
        self.catalog.albums().await
    }

    pub async fn tracks(&self) -> Result<Vec<Track>, LibraryError> {
        self.catalog.tracks().await
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        self.catalog.artist(id).await
    }

    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        self.catalog.album(id).await
    }

    pub async fn cover(&self, id: &CoverId, px: u32) -> Result<Bytes, LibraryError> {
        self.catalog.cover(id, px).await
    }

    pub async fn track_bytes(&self, id: &TrackId) -> Result<Bytes, LibraryError> {
        self.catalog.track_bytes(id).await
    }

    // --- Global database (playlists and history) --------------------------

    pub async fn playlists(&self) -> Result<Vec<Playlist>, LibraryError> {
        self.catalog.global.playlists().await
    }

    pub async fn create_playlist(&self, name: &str) -> Result<Playlist, LibraryError> {
        self.catalog.global.create_playlist(name).await
    }

    pub async fn rename_playlist(&self, id: &str, name: &str) -> Result<(), LibraryError> {
        self.catalog.global.rename_playlist(id, name).await
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<(), LibraryError> {
        self.catalog.global.delete_playlist(id).await
    }

    pub async fn playlist_tracks(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<PlaylistTrack>, LibraryError> {
        self.catalog.global.playlist_tracks(playlist_id).await
    }

    pub async fn add_playlist_track(
        &self,
        playlist_id: &str,
        track: &Track,
    ) -> Result<(), LibraryError> {
        self.catalog
            .global
            .add_playlist_track(playlist_id, track)
            .await
    }

    pub async fn remove_playlist_track(
        &self,
        playlist_id: &str,
        position: i64,
    ) -> Result<(), LibraryError> {
        self.catalog
            .global
            .remove_playlist_track(playlist_id, position)
            .await
    }

    pub async fn move_playlist_track(
        &self,
        playlist_id: &str,
        from: i64,
        to: i64,
    ) -> Result<(), LibraryError> {
        self.catalog
            .global
            .move_playlist_track(playlist_id, from, to)
            .await
    }

    pub async fn record_play(
        &self,
        track_id: &TrackId,
        started_at: i64,
        played_ms: Option<i64>,
        completed: bool,
    ) -> Result<(), LibraryError> {
        self.catalog
            .global
            .record_play(track_id, started_at, played_ms, completed)
            .await
    }

    pub async fn play_history(&self, limit: i64) -> Result<Vec<PlayHistoryEntry>, LibraryError> {
        self.catalog.global.play_history(limit).await
    }
}

impl ManagerHandle {
    /// `(identifier, root)` for every opened library. Used for watcher
    /// subscriptions, which only need the path.
    pub fn watchers(&self) -> Vec<(String, std::path::PathBuf)> {
        self.0
            .libraries
            .iter()
            .map(|opened| {
                let root = match &opened.config {
                    LibraryConfig::Offline { root, .. } => root.clone(),
                };
                (opened.config.identifier().to_string(), root)
            })
            .collect()
    }

    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        self.0.artists().await
    }

    pub async fn albums(&self) -> Result<Vec<Album>, LibraryError> {
        self.0.albums().await
    }

    pub async fn tracks(&self) -> Result<Vec<Track>, LibraryError> {
        self.0.tracks().await
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        self.0.artist(id).await
    }

    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        self.0.album(id).await
    }

    pub async fn cover(&self, id: &CoverId, px: u32) -> Result<Bytes, LibraryError> {
        self.0.cover(id, px).await
    }

    pub async fn track_bytes(&self, id: &TrackId) -> Result<Bytes, LibraryError> {
        self.0.track_bytes(id).await
    }
}

impl Catalog {
    async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        let mut artists = Vec::new();
        for opened in &self.libraries {
            let prefix = opened.config.identifier();
            for mut artist in opened.library.artists().await? {
                re_prefix_artist(prefix, &mut artist);
                artists.push(artist);
            }
        }
        artists.sort_by_key(|a| a.name.to_lowercase());
        Ok(artists)
    }

    async fn albums(&self) -> Result<Vec<Album>, LibraryError> {
        let mut albums = Vec::new();
        for opened in &self.libraries {
            let prefix = opened.config.identifier();
            for mut album in opened.library.albums().await? {
                re_prefix_album(prefix, &mut album);
                albums.push(album);
            }
        }
        albums.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| {
                    a.artist
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .cmp(&b.artist.as_deref().unwrap_or_default().to_lowercase())
                })
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        Ok(albums)
    }

    async fn tracks(&self) -> Result<Vec<Track>, LibraryError> {
        let mut tracks = Vec::new();
        for opened in &self.libraries {
            let prefix = opened.config.identifier();
            for mut track in opened.library.tracks().await? {
                re_prefix_track(prefix, &mut track);
                tracks.push(track);
            }
        }
        tracks.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| {
                    a.artist
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .cmp(&b.artist.as_deref().unwrap_or_default().to_lowercase())
                })
                .then_with(|| {
                    a.album
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .cmp(&b.album.as_deref().unwrap_or_default().to_lowercase())
                })
                .then_with(|| a.track_no.cmp(&b.track_no))
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        Ok(tracks)
    }

    async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        let (opened, rest) = self.route(id.as_str())?;
        let mut detail = opened.library.artist(&ArtistId::new(rest)).await?;
        let prefix = opened.config.identifier();
        re_prefix_artist(prefix, &mut detail.artist);
        for album in &mut detail.albums {
            re_prefix_album(prefix, album);
        }
        Ok(detail)
    }

    async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        let (opened, rest) = self.route(id.as_str())?;
        let mut detail = opened.library.album(&AlbumId::new(rest)).await?;
        let prefix = opened.config.identifier();
        re_prefix_album(prefix, &mut detail.album);
        for song in &mut detail.songs {
            re_prefix_track(prefix, song);
        }
        Ok(detail)
    }

    async fn cover(&self, id: &CoverId, px: u32) -> Result<Bytes, LibraryError> {
        let (opened, rest) = self.route(id.as_str())?;
        opened.library.cover(&CoverId::new(rest), px).await
    }

    async fn track_bytes(&self, id: &TrackId) -> Result<Bytes, LibraryError> {
        let (opened, rest) = self.route(id.as_str())?;
        opened.library.track_bytes(&TrackId::new(rest)).await
    }

    fn scan_plan(&self) -> Vec<ScanPlan> {
        self.libraries
            .iter()
            .map(|opened| ScanPlan {
                library: opened.config.identifier().to_string(),
                config: opened.config.clone(),
                handle: match &opened.library {
                    Library::Offline(offline) => LibraryHandle::Offline(offline.handle()),
                },
            })
            .collect()
    }

    fn route<'a>(&'a self, id: &str) -> Result<(&'a Opened, String), LibraryError> {
        let Some((prefix, rest)) = id.split_once(':') else {
            return Err(LibraryError::NotFound(format!(
                "unprefixed library id: {id}"
            )));
        };
        let opened = self
            .libraries
            .iter()
            .find(|o| o.config.identifier() == prefix)
            .ok_or_else(|| {
                LibraryError::NotFound(format!("unknown library identifier: {prefix}"))
            })?;
        Ok((opened, rest.to_string()))
    }
}

impl LibraryHandle {
    pub fn identifier(&self) -> &str {
        match self {
            Self::Offline(handle) => &handle.identifier,
        }
    }

    pub fn root(&self) -> &Path {
        match self {
            Self::Offline(handle) => &handle.root,
        }
    }

    pub fn pool(&self) -> &SqlitePool {
        match self {
            Self::Offline(handle) => &handle.pool,
        }
    }
}

fn re_prefix_artist(prefix: &str, artist: &mut Artist) {
    artist.id = ArtistId::new(format!("{prefix}:{}", artist.id.as_str()));
    if let Some(cover) = &artist.cover {
        artist.cover = Some(CoverId::new(format!("{prefix}:{}", cover.as_str())));
    }
}

fn re_prefix_album(prefix: &str, album: &mut Album) {
    album.id = AlbumId::new(format!("{prefix}:{}", album.id.as_str()));
    if let Some(cover) = &album.cover {
        album.cover = Some(CoverId::new(format!("{prefix}:{}", cover.as_str())));
    }
}

fn re_prefix_track(prefix: &str, track: &mut Track) {
    track.id = TrackId::new(format!("{prefix}:{}", track.id.as_str()));
    if let Some(cover) = &track.cover {
        track.cover = Some(CoverId::new(format!("{prefix}:{}", cover.as_str())));
    }
}
