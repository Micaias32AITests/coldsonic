//! SQLite plumbing shared by the global and per-library databases.
//!
//! Both databases live under the app's `data_dir()`. Library databases are
//! derived data and are recreated when their schema version changes; the
//! global database holds durable user data and runs sqlx migrations.

pub mod global;
pub mod library;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::library::LibraryError;

/// Pools are small: SQLite has a single writer and WAL readers are cheap.
const MAX_CONNECTIONS: u32 = 4;

pub fn global_db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("global.sqlite")
}

pub fn library_db_path(data_dir: &Path, identifier: &str) -> PathBuf {
    data_dir
        .join("libraries")
        .join(format!("{identifier}.sqlite"))
}

pub fn library_cache_dir(cache_dir: &Path, identifier: &str) -> PathBuf {
    cache_dir.join("libraries").join(identifier)
}

pub fn library_cover_cache_dir(cache_dir: &Path, identifier: &str) -> PathBuf {
    library_cache_dir(cache_dir, identifier).join("covers")
}

/// Open (and optionally create) a SQLite pool with the pragmas every
/// coldsonic database uses.
pub async fn open_pool(path: &Path, create_if_missing: bool) -> Result<SqlitePool, LibraryError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // `filename` takes the path directly, so paths with spaces or other
    // URL-hostile characters work without escaping.
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(create_if_missing)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(5_000));

    let pool = SqlitePoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .connect_with(options)
        .await?;
    Ok(pool)
}
