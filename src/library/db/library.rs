//! Per-library metadata database.
//!
//! One SQLite file per library under `data_dir()/libraries/`. The database is
//! derived data: if the schema version changes we drop and recreate it, and a
//! scan signature mismatch forces a rescan. All ids stored here are
//! unprefixed; the manager adds prefixes on the way out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::{Row, SqlitePool};

use crate::library::db::open_pool;
use crate::library::scan::ScanData;
use crate::library::{
    Album, AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, CoverId, LibraryConfig,
    LibraryError, ScanMode, Track, TrackId,
};

pub const SCHEMA_VERSION: &str = "1";
pub const SCANNER_VERSION: &str = "1";

const TABLES: [&str; 5] = ["files", "tracks", "albums", "artists", "covers"];

const SCHEMA: [&str; 14] = [
    "CREATE TABLE IF NOT EXISTS meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS artists (
        id          TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        sort_name   TEXT,
        album_count INTEGER NOT NULL DEFAULT 0,
        cover_id    TEXT
    )",
    "CREATE TABLE IF NOT EXISTS covers (
        id          TEXT PRIMARY KEY,
        kind        TEXT NOT NULL,
        source_path TEXT NOT NULL,
        cache_path  TEXT,
        mime        TEXT,
        mtime_ns    INTEGER,
        size        INTEGER
    )",
    "CREATE TABLE IF NOT EXISTS albums (
        id          TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        artist_id   TEXT REFERENCES artists(id) ON DELETE SET NULL,
        artist_name TEXT,
        song_count  INTEGER NOT NULL DEFAULT 0,
        duration_ms INTEGER,
        cover_id    TEXT REFERENCES covers(id) ON DELETE SET NULL
    )",
    "CREATE TABLE IF NOT EXISTS tracks (
        id          TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        artist_id   TEXT REFERENCES artists(id) ON DELETE SET NULL,
        artist_name TEXT,
        album_id    TEXT REFERENCES albums(id) ON DELETE SET NULL,
        album_name  TEXT,
        track_no    INTEGER,
        duration_ms INTEGER NOT NULL,
        cover_id    TEXT REFERENCES covers(id) ON DELETE SET NULL
    )",
    "CREATE TABLE IF NOT EXISTS files (
        id       INTEGER PRIMARY KEY,
        path     TEXT NOT NULL UNIQUE,
        mtime_ns INTEGER NOT NULL,
        size     INTEGER NOT NULL,
        kind     TEXT NOT NULL,
        track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL
    )",
    "CREATE INDEX IF NOT EXISTS albums_name_idx   ON albums(name)",
    "CREATE INDEX IF NOT EXISTS tracks_title_idx  ON tracks(title)",
    "CREATE INDEX IF NOT EXISTS tracks_album_idx  ON tracks(album_id)",
    "CREATE INDEX IF NOT EXISTS tracks_artist_idx ON tracks(artist_id)",
    "CREATE INDEX IF NOT EXISTS files_track_idx   ON files(track_id)",
    "CREATE INDEX IF NOT EXISTS artists_name_idx  ON artists(name)",
    "CREATE INDEX IF NOT EXISTS covers_source_idx ON covers(source_path)",
    "CREATE INDEX IF NOT EXISTS tracks_cover_idx  ON tracks(cover_id)",
];

#[derive(Debug, Clone)]
pub struct LibraryDb {
    pub pool: SqlitePool,
}

/// Stable per-library scan inputs that must match for a completed scan to be
/// reusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSignature {
    pub scan_mode: String,
    pub never_split_hash: String,
    pub root: String,
    pub identifier: String,
    pub scanner_version: String,
}

impl ScanSignature {
    pub fn from_config(config: &LibraryConfig) -> Self {
        let LibraryConfig::Offline {
            id,
            root,
            scan,
            never_split,
            ..
        } = config;
        let mode = match scan {
            ScanMode::Tags => "tags",
            ScanMode::Folders => "folders",
        };
        let mut words: Vec<String> = never_split
            .iter()
            .map(|word| word.trim().to_lowercase())
            .collect();
        words.sort();
        Self {
            scan_mode: mode.to_string(),
            never_split_hash: stable_hash(&words.join("\u{0}")),
            root: root.to_string_lossy().into_owned(),
            identifier: id.clone(),
            scanner_version: SCANNER_VERSION.to_string(),
        }
    }
}

/// One `files` row used by the startup stat walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub mtime_ns: i64,
    pub size: i64,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverRecord {
    pub kind: CoverKind,
    pub source_path: String,
    pub cache_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverKind {
    Folder,
    Embedded,
}

impl CoverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Embedded => "embedded",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "embedded" => Self::Embedded,
            _ => Self::Folder,
        }
    }
}

impl LibraryDb {
    pub async fn open(path: &Path) -> Result<Self, LibraryError> {
        let pool = open_pool(path, true).await?;
        let db = Self { pool };
        db.ensure_schema().await?;
        Ok(db)
    }

    /// Create the schema, recreating it when `SCHEMA_VERSION` changed.
    pub async fn ensure_schema(&self) -> Result<(), LibraryError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        let version: Option<String> =
            sqlx::query_scalar("SELECT value FROM meta WHERE key = 'schema_version'")
                .fetch_optional(&self.pool)
                .await?;

        if version
            .as_deref()
            .is_some_and(|value| value != SCHEMA_VERSION)
        {
            for table in TABLES {
                sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
                    .execute(&self.pool)
                    .await?;
            }
        }

        for statement in SCHEMA {
            sqlx::query(statement).execute(&self.pool).await?;
        }

        self.set_meta("schema_version", SCHEMA_VERSION).await?;
        Ok(())
    }

    /// Compare the stored scan signature with the expected one and clear
    /// `scan_complete` on a mismatch.
    pub async fn apply_signature(&self, signature: &ScanSignature) -> Result<(), LibraryError> {
        let entries = [
            ("scan_mode", signature.scan_mode.as_str()),
            ("never_split_hash", signature.never_split_hash.as_str()),
            ("root", signature.root.as_str()),
            ("identifier", signature.identifier.as_str()),
            ("scanner_version", signature.scanner_version.as_str()),
        ];

        let mut mismatch = false;
        for (key, value) in entries {
            let stored: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
            if stored.as_deref() != Some(value) {
                mismatch = true;
                self.set_meta(key, value).await?;
            }
        }

        if mismatch {
            self.set_meta("scan_complete", "0").await?;
        }
        Ok(())
    }

    pub async fn scan_complete(&self) -> Result<bool, LibraryError> {
        let complete: Option<String> =
            sqlx::query_scalar("SELECT value FROM meta WHERE key = 'scan_complete'")
                .fetch_optional(&self.pool)
                .await?;
        Ok(complete.as_deref() == Some("1"))
    }

    pub async fn files(&self) -> Result<HashMap<String, FileStat>, LibraryError> {
        let rows = sqlx::query("SELECT path, mtime_ns, size, kind FROM files")
            .fetch_all(&self.pool)
            .await?;
        let mut files = HashMap::with_capacity(rows.len());
        for row in rows {
            files.insert(
                row.try_get("path")?,
                FileStat {
                    mtime_ns: row.try_get("mtime_ns")?,
                    size: row.try_get("size")?,
                    kind: row.try_get("kind")?,
                },
            );
        }
        Ok(files)
    }

    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        let rows = sqlx::query(
            "SELECT id, name, album_count, cover_id FROM artists \
             ORDER BY name COLLATE NOCASE, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(artist_from_row).collect()
    }

    pub async fn albums(&self) -> Result<Vec<Album>, LibraryError> {
        let rows = sqlx::query(
            "SELECT id, name, artist_name, song_count, duration_ms, cover_id FROM albums \
             ORDER BY name COLLATE NOCASE, artist_name COLLATE NOCASE, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(album_from_row).collect()
    }

    pub async fn tracks(&self) -> Result<Vec<Track>, LibraryError> {
        let rows = sqlx::query(
            "SELECT id, title, artist_name, album_name, track_no, duration_ms, cover_id \
             FROM tracks \
             ORDER BY title COLLATE NOCASE, artist_name COLLATE NOCASE, \
                      album_name COLLATE NOCASE, track_no IS NULL, track_no, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(track_from_row).collect()
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        let mut tx = self.pool.begin_with("BEGIN DEFERRED").await?;

        let row = sqlx::query("SELECT id, name, album_count, cover_id FROM artists WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| LibraryError::NotFound(format!("no artist {}", id.as_str())))?;
        let artist = artist_from_row(row)?;

        let rows = sqlx::query(
            "SELECT id, name, artist_name, song_count, duration_ms, cover_id FROM albums \
             WHERE artist_id = ? ORDER BY name COLLATE NOCASE, id",
        )
        .bind(id.as_str())
        .fetch_all(&mut *tx)
        .await?;
        let albums = rows
            .into_iter()
            .map(album_from_row)
            .collect::<Result<Vec<_>, _>>()?;

        tx.commit().await?;
        Ok(ArtistDetail { artist, albums })
    }

    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        let mut tx = self.pool.begin_with("BEGIN DEFERRED").await?;

        let row = sqlx::query(
            "SELECT id, name, artist_name, song_count, duration_ms, cover_id FROM albums \
             WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| LibraryError::NotFound(format!("no album {}", id.as_str())))?;
        let album = album_from_row(row)?;

        let rows = sqlx::query(
            "SELECT id, title, artist_name, album_name, track_no, duration_ms, cover_id \
             FROM tracks WHERE album_id = ? \
             ORDER BY track_no IS NULL, track_no, id",
        )
        .bind(id.as_str())
        .fetch_all(&mut *tx)
        .await?;
        let songs = rows
            .into_iter()
            .map(track_from_row)
            .collect::<Result<Vec<_>, _>>()?;

        tx.commit().await?;
        Ok(AlbumDetail { album, songs })
    }

    pub async fn cover(&self, id: &CoverId) -> Result<CoverRecord, LibraryError> {
        let row = sqlx::query("SELECT kind, source_path, cache_path FROM covers WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| LibraryError::NotFound(format!("no cover {}", id.as_str())))?;
        Ok(CoverRecord {
            kind: CoverKind::from_str(&row.try_get::<String, _>("kind")?),
            source_path: row.try_get("source_path")?,
            cache_path: row.try_get("cache_path")?,
        })
    }

    /// Replace every metadata row in one short transaction. All filesystem and
    /// tag work must already be complete. Embedded cover bytes are written to
    /// the cache before the transaction opens.
    pub async fn write_scan(
        &self,
        data: &ScanData,
        cache_dir: &Path,
        signature: &ScanSignature,
    ) -> Result<(), LibraryError> {
        tokio::fs::create_dir_all(cache_dir).await?;
        let mut cache_paths: HashMap<String, String> = HashMap::new();
        for cover in &data.covers {
            let Some(bytes) = &cover.bytes else {
                continue;
            };
            let file_name = cache_file_name(&cover.id, cover.mime.as_deref());
            let path = cache_dir.join(&file_name);
            if let Err(err) = tokio::fs::write(&path, bytes).await {
                // A read-only cache must not fail the whole library; the cover
                // is simply unavailable until the next successful scan.
                tracing::warn!(path = %path.display(), %err, "failed to cache embedded cover");
                continue;
            }
            cache_paths.insert(cover.id.clone(), file_name);
        }

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        for table in ["files", "tracks", "albums", "artists", "covers"] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *tx)
                .await?;
        }

        for artist in &data.artists {
            sqlx::query(
                "INSERT INTO artists (id, name, sort_name, album_count, cover_id) \
                 VALUES (?, ?, NULL, ?, ?)",
            )
            .bind(&artist.name)
            .bind(&artist.name)
            .bind(artist.album_count)
            .bind(artist.cover.as_deref())
            .execute(&mut *tx)
            .await?;
        }

        for cover in &data.covers {
            sqlx::query(
                "INSERT INTO covers (id, kind, source_path, cache_path, mime, mtime_ns, size) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&cover.id)
            .bind(cover.kind.as_str())
            .bind(&cover.source_path)
            .bind(cache_paths.get(&cover.id))
            .bind(cover.mime.as_deref())
            .bind(cover.mtime_ns)
            .bind(cover.size)
            .execute(&mut *tx)
            .await?;
        }

        for album in &data.albums {
            sqlx::query(
                "INSERT INTO albums \
                 (id, name, artist_id, artist_name, song_count, duration_ms, cover_id) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&album.id)
            .bind(&album.name)
            .bind(&album.artist)
            .bind(&album.artist)
            .bind(album.song_count)
            .bind(album.duration.map(duration_ms))
            .bind(album.cover.as_deref())
            .execute(&mut *tx)
            .await?;
        }

        for track in &data.tracks {
            sqlx::query(
                "INSERT INTO tracks \
                 (id, title, artist_id, artist_name, album_id, album_name, track_no, \
                  duration_ms, cover_id) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&track.id)
            .bind(&track.title)
            .bind(&track.artist)
            .bind(&track.artist)
            .bind(&track.album_id)
            .bind(&track.album)
            .bind(track.track_no.map(i64::from))
            .bind(duration_ms(track.duration))
            .bind(track.cover.as_deref())
            .execute(&mut *tx)
            .await?;
        }

        for file in &data.files {
            sqlx::query(
                "INSERT INTO files (path, mtime_ns, size, kind, track_id) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&file.path)
            .bind(file.mtime_ns)
            .bind(file.size)
            .bind(file.kind.as_str())
            .bind(file.track_id.as_deref())
            .execute(&mut *tx)
            .await?;
        }

        let entries = [
            ("schema_version", SCHEMA_VERSION.to_string()),
            ("scanner_version", signature.scanner_version.clone()),
            ("scan_mode", signature.scan_mode.clone()),
            ("never_split_hash", signature.never_split_hash.clone()),
            ("root", signature.root.clone()),
            ("identifier", signature.identifier.clone()),
            ("scan_complete", "1".to_string()),
            ("last_scan_unix_nanos", unix_nanos().to_string()),
        ];
        for (key, value) in entries {
            sqlx::query(
                "INSERT INTO meta (key, value) VALUES (?, ?) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn set_meta(&self, key: &str, value: &str) -> Result<(), LibraryError> {
        sqlx::query(
            "INSERT INTO meta (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn artist_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Artist, LibraryError> {
    Ok(Artist {
        id: ArtistId::new(row.try_get::<String, _>("id")?),
        name: row.try_get("name")?,
        album_count: row.try_get::<i64, _>("album_count")? as u32,
        cover: row
            .try_get::<Option<String>, _>("cover_id")?
            .map(CoverId::new),
    })
}

fn album_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Album, LibraryError> {
    Ok(Album {
        id: AlbumId::new(row.try_get::<String, _>("id")?),
        name: row.try_get("name")?,
        artist: row.try_get("artist_name")?,
        song_count: row.try_get::<i64, _>("song_count")? as u32,
        duration: row
            .try_get::<Option<i64>, _>("duration_ms")?
            .map(|ms| Duration::from_millis(ms.max(0) as u64)),
        cover: row
            .try_get::<Option<String>, _>("cover_id")?
            .map(CoverId::new),
    })
}

fn track_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Track, LibraryError> {
    Ok(Track {
        id: TrackId::new(row.try_get::<String, _>("id")?),
        title: row.try_get("title")?,
        artist: row.try_get("artist_name")?,
        album: row.try_get("album_name")?,
        track_no: row
            .try_get::<Option<i64>, _>("track_no")?
            .map(|no| no.max(0) as u32),
        duration: Duration::from_millis(row.try_get::<i64, _>("duration_ms")?.max(0) as u64),
        cover: row
            .try_get::<Option<String>, _>("cover_id")?
            .map(CoverId::new),
    })
}

fn duration_ms(duration: Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}

fn cache_file_name(id: &str, mime: Option<&str>) -> String {
    let extension = match mime {
        Some("image/png") => "png",
        Some("image/webp") => "webp",
        Some("image/gif") => "gif",
        _ => "jpg",
    };
    format!("{}.{extension}", stable_hash(id))
}

fn unix_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

/// FNV-1a. Stable across runs and Rust releases, unlike `DefaultHasher`.
pub fn stable_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Resolve a folder-art path relative to the library root. Kept here so the
/// provider and the scanner use the same path shape.
pub fn resolve(root: &Path, relative: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path
}
