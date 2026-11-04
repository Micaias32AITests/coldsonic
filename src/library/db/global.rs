//! The global database: playlists and play history.
//!
//! This database is separate from every library database so playlist writes
//! never contend with a scan. Track references are stored **prefixed**; the
//! snapshot columns let the UI render an entry whose library/track is gone.
//! Playlist UI is out of scope for this change; the schema and manager methods
//! land here.

use std::path::Path;

use sqlx::{Row, SqlitePool};

use crate::library::db::open_pool;
use crate::library::{LibraryError, PlayHistoryEntry, Playlist, PlaylistTrack, Track, TrackId};

#[derive(Debug, Clone)]
pub struct GlobalDb {
    pool: SqlitePool,
}

impl GlobalDb {
    pub async fn open(path: &Path) -> Result<Self, LibraryError> {
        let pool = open_pool(path, true).await?;
        sqlx::migrate!("./migrations/global")
            .run(&pool)
            .await
            .map_err(|err| LibraryError::Db(err.into()))?;
        Ok(Self { pool })
    }

    pub async fn playlists(&self) -> Result<Vec<Playlist>, LibraryError> {
        let rows = sqlx::query(
            "SELECT id, name, created_at, updated_at FROM playlists ORDER BY name COLLATE NOCASE, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(Playlist {
                    id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }

    pub async fn create_playlist(&self, name: &str) -> Result<Playlist, LibraryError> {
        let now = unix_seconds();
        let id = new_id(now);
        sqlx::query("INSERT INTO playlists (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(name)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(Playlist {
            id,
            name: name.to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn rename_playlist(&self, id: &str, name: &str) -> Result<(), LibraryError> {
        let result = sqlx::query("UPDATE playlists SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(unix_seconds())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(LibraryError::NotFound(format!("no playlist {id}")));
        }
        Ok(())
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<(), LibraryError> {
        sqlx::query("DELETE FROM playlists WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn playlist_tracks(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<PlaylistTrack>, LibraryError> {
        let rows = sqlx::query(
            "SELECT playlist_id, position, track_id, library_id, title, artist, album, added_at \
             FROM playlist_tracks WHERE playlist_id = ? ORDER BY position",
        )
        .bind(playlist_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PlaylistTrack {
                    playlist_id: row.try_get("playlist_id")?,
                    position: row.try_get("position")?,
                    track_id: TrackId::new(row.try_get::<String, _>("track_id")?),
                    library_id: row.try_get("library_id")?,
                    title: row.try_get("title")?,
                    artist: row.try_get("artist")?,
                    album: row.try_get("album")?,
                    added_at: row.try_get("added_at")?,
                })
            })
            .collect()
    }

    pub async fn add_playlist_track(
        &self,
        playlist_id: &str,
        track: &Track,
    ) -> Result<(), LibraryError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM playlists WHERE id = ?")
            .bind(playlist_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(LibraryError::NotFound(format!("no playlist {playlist_id}")));
        }

        let position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_tracks WHERE playlist_id = ?",
        )
        .bind(playlist_id)
        .fetch_one(&mut *tx)
        .await?;

        let (library_id, _) = split_prefixed(track.id.as_str())?;
        sqlx::query(
            "INSERT INTO playlist_tracks \
             (playlist_id, position, track_id, library_id, title, artist, album, added_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(playlist_id)
        .bind(position)
        .bind(track.id.as_str())
        .bind(library_id)
        .bind(&track.title)
        .bind(&track.artist)
        .bind(&track.album)
        .bind(unix_seconds())
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE playlists SET updated_at = ? WHERE id = ?")
            .bind(unix_seconds())
            .bind(playlist_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_playlist_track(
        &self,
        playlist_id: &str,
        position: i64,
    ) -> Result<(), LibraryError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ? AND position = ?")
            .bind(playlist_id)
            .bind(position)
            .execute(&mut *tx)
            .await?;
        compact_positions(&mut tx, playlist_id).await?;
        sqlx::query("UPDATE playlists SET updated_at = ? WHERE id = ?")
            .bind(unix_seconds())
            .bind(playlist_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn move_playlist_track(
        &self,
        playlist_id: &str,
        from: i64,
        to: i64,
    ) -> Result<(), LibraryError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let mut snapshots = playlist_snapshots(&mut tx, playlist_id).await?;
        if snapshots.is_empty() {
            return Err(LibraryError::NotFound(format!(
                "playlist {playlist_id} has no tracks"
            )));
        }

        let from = (from.max(0) as usize).min(snapshots.len() - 1);
        let to = (to.max(0) as usize).min(snapshots.len() - 1);
        let moved = snapshots.remove(from);
        snapshots.insert(to, moved);

        sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ?")
            .bind(playlist_id)
            .execute(&mut *tx)
            .await?;
        insert_snapshots(&mut tx, playlist_id, snapshots).await?;

        sqlx::query("UPDATE playlists SET updated_at = ? WHERE id = ?")
            .bind(unix_seconds())
            .bind(playlist_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn record_play(
        &self,
        track_id: &TrackId,
        started_at: i64,
        played_ms: Option<i64>,
        completed: bool,
    ) -> Result<(), LibraryError> {
        let (library_id, _) = split_prefixed(track_id.as_str())?;
        sqlx::query(
            "INSERT INTO play_history (track_id, library_id, started_at, played_ms, completed) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(track_id.as_str())
        .bind(library_id)
        .bind(started_at)
        .bind(played_ms)
        .bind(completed as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn play_history(&self, limit: i64) -> Result<Vec<PlayHistoryEntry>, LibraryError> {
        let rows = sqlx::query(
            "SELECT id, track_id, library_id, started_at, played_ms, completed \
             FROM play_history ORDER BY started_at DESC, id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PlayHistoryEntry {
                    id: row.try_get("id")?,
                    track_id: TrackId::new(row.try_get::<String, _>("track_id")?),
                    library_id: row.try_get("library_id")?,
                    started_at: row.try_get("started_at")?,
                    played_ms: row.try_get("played_ms")?,
                    completed: row.try_get::<i64, _>("completed")? != 0,
                })
            })
            .collect()
    }
}

async fn compact_positions(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    playlist_id: &str,
) -> Result<(), LibraryError> {
    let snapshots = playlist_snapshots(tx, playlist_id).await?;
    sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ?")
        .bind(playlist_id)
        .execute(&mut **tx)
        .await?;
    insert_snapshots(tx, playlist_id, snapshots).await
}

struct Snapshot {
    track_id: String,
    library_id: String,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    added_at: i64,
}

async fn playlist_snapshots(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    playlist_id: &str,
) -> Result<Vec<Snapshot>, LibraryError> {
    let rows = sqlx::query(
        "SELECT track_id, library_id, title, artist, album, added_at FROM playlist_tracks \
         WHERE playlist_id = ? ORDER BY position",
    )
    .bind(playlist_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(Snapshot {
                track_id: row.try_get("track_id")?,
                library_id: row.try_get("library_id")?,
                title: row.try_get("title")?,
                artist: row.try_get("artist")?,
                album: row.try_get("album")?,
                added_at: row.try_get("added_at")?,
            })
        })
        .collect()
}

async fn insert_snapshots(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    playlist_id: &str,
    snapshots: Vec<Snapshot>,
) -> Result<(), LibraryError> {
    for (position, snapshot) in snapshots.into_iter().enumerate() {
        sqlx::query(
            "INSERT INTO playlist_tracks \
             (playlist_id, position, track_id, library_id, title, artist, album, added_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(playlist_id)
        .bind(position as i64)
        .bind(snapshot.track_id)
        .bind(snapshot.library_id)
        .bind(snapshot.title)
        .bind(snapshot.artist)
        .bind(snapshot.album)
        .bind(snapshot.added_at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn split_prefixed(id: &str) -> Result<(&str, &str), LibraryError> {
    id.split_once(':')
        .ok_or_else(|| LibraryError::Invalid(format!("unprefixed track id: {id}")))
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn new_id(now: i64) -> String {
    let random: u64 = rand::random();
    format!("{now:x}-{random:016x}")
}
