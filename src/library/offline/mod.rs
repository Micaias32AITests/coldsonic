//! Offline provider — local files backed by a per-library SQLite database.
//!
//! Opening a library creates/opens its database and applies the scan
//! signature, but performs no filesystem walk. All metadata reads are SQL
//! queries; audio bytes and folder art are read directly from the library
//! root, and only extracted embedded art comes from the per-library cache.

use std::path::PathBuf;

use bytes::Bytes;
use sqlx::SqlitePool;

use crate::library::db::library::{CoverKind, LibraryDb, resolve};
use crate::library::{
    Album, AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, CoverId, LibraryError, ScanMode,
    Track, TrackId,
};

/// Everything an async task needs to query one offline library.
#[derive(Debug, Clone)]
pub struct OfflineHandle {
    pub(crate) identifier: String,
    pub(crate) root: PathBuf,
    pub(crate) pool: SqlitePool,
    /// Per-library cover cache directory (`.../{identifier}/covers`).
    pub(crate) cache_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct OfflineLibrary {
    handle: OfflineHandle,
    /// Kept for scans; not read by queries.
    #[allow(dead_code)]
    mode: ScanMode,
    #[allow(dead_code)]
    never_split: Vec<String>,
}

impl OfflineLibrary {
    pub(crate) fn new(handle: OfflineHandle, mode: ScanMode, never_split: Vec<String>) -> Self {
        Self {
            handle,
            mode,
            never_split,
        }
    }

    pub fn handle(&self) -> OfflineHandle {
        self.handle.clone()
    }

    fn db(&self) -> LibraryDb {
        LibraryDb {
            pool: self.handle.pool.clone(),
        }
    }

    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        self.db().artists().await
    }

    pub async fn albums(&self) -> Result<Vec<Album>, LibraryError> {
        self.db().albums().await
    }

    pub async fn tracks(&self) -> Result<Vec<Track>, LibraryError> {
        self.db().tracks().await
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        self.db().artist(id).await
    }

    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        self.db().album(id).await
    }

    /// Offline providers do not resize; `px` stays in the signature for
    /// online providers.
    pub async fn cover(&self, id: &CoverId, _px: u32) -> Result<Bytes, LibraryError> {
        let record = self.db().cover(id).await?;
        let path = match record.kind {
            CoverKind::Folder => resolve(&self.handle.root, &record.source_path),
            CoverKind::Embedded => {
                let Some(cache_path) = record.cache_path else {
                    return Err(LibraryError::NotFound(format!(
                        "cover {} has no cached blob",
                        id.as_str()
                    )));
                };
                self.handle.cache_dir.join(cache_path)
            }
        };
        let data = tokio::fs::read(&path).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                LibraryError::NotFound(format!("cover file {}", path.display()))
            } else {
                LibraryError::Io(err)
            }
        })?;
        Ok(Bytes::from(data))
    }

    /// The track id is the relative path, so no database query is needed.
    pub async fn track_bytes(&self, id: &TrackId) -> Result<Bytes, LibraryError> {
        let path = resolve(&self.handle.root, id.as_str());
        let data = tokio::fs::read(&path).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                LibraryError::NotFound(format!("track file {}", path.display()))
            } else {
                LibraryError::Io(err)
            }
        })?;
        Ok(Bytes::from(data))
    }
}
