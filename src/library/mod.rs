//! Media libraries: neutral types, the provider seam, and config.
//!
//! coldsonic can play songs from many kinds of sources — offline files today,
//! subsonic next, smb/youtube someday. This module draws the line: screens
//! hold an [`Manager`] (never a provider), providers speak only the neutral
//! types below ([`Track`], [`Artist`], [`Album`], covers as [`Bytes`]), and
//! foreign types (subsonic's `Child`, …) never cross the seam. Ids are opaque
//! newtypes ([`TrackId`], [`ArtistId`], [`AlbumId`], [`CoverId`]) that the
//! manager prefixes with the library's identifier (`{identifier}:`) so routing
//! a query back down is pure string work with no guessing.

pub mod manager;
pub mod offline;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

pub use manager::Manager;

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            pub(crate) fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

id_newtype!(
    /// Opaque track id. Offline: path relative to the library root, `/`
    /// separators. The manager prefixes it with `{identifier}:`.
    TrackId
);
id_newtype!(
    /// Opaque artist id. Offline: the artist name itself.
    ArtistId
);
id_newtype!(
    /// Opaque album id. Offline: `artist\u{0}album`.
    AlbumId
);
id_newtype!(
    /// Opaque cover-art id. Offline: album directory (folder art) or track
    /// path (embedded art).
    CoverId
);

/// A playable song, as every provider hands it over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub duration: Duration,
    pub cover: Option<CoverId>,
}

/// A top-level artist entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artist {
    pub id: ArtistId,
    pub name: String,
    pub album_count: u32,
}

/// An artist plus the albums under them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistDetail {
    pub artist: Artist,
    pub albums: Vec<Album>,
}

/// An album under an artist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Album {
    pub id: AlbumId,
    pub name: String,
    pub artist: Option<String>,
    pub song_count: u32,
    pub duration: Option<Duration>,
    pub cover: Option<CoverId>,
}

/// An album plus its songs, in play order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumDetail {
    pub album: Album,
    pub songs: Vec<Track>,
}

/// Errors surfaced by the library seam.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

/// The provider seam — an enum wrapper (mirroring
/// [`Engine`](crate::engine::Engine)) so screens keep one import and every new
/// provider marks its touchpoints with exhaustive matches. Ids arriving here
/// are unprefixed; the manager owns prefixing.
#[derive(Debug, Clone)]
pub enum Library {
    Offline(offline::OfflineLibrary),
}

impl Library {
    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        match self {
            Library::Offline(library) => library.artists().await,
        }
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        match self {
            Library::Offline(library) => library.artist(id).await,
        }
    }

    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        match self {
            Library::Offline(library) => library.album(id).await,
        }
    }

    pub async fn cover(&self, id: &CoverId, px: u32) -> Result<Bytes, LibraryError> {
        match self {
            Library::Offline(library) => library.cover(id, px).await,
        }
    }
}

/// How an offline library derives names from its files.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    /// Names from embedded tags; covers from embedded pictures.
    #[default]
    Tags,
    /// Names from the `Artist/Album/NN - Title.ext` directory structure;
    /// covers from folder-art files, else embedded pictures.
    Folders,
}

/// One entry of `libraries.toml`. The manager consumes it; `config.rs` only
/// does file IO.
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
}

impl LibraryConfig {
    /// The stable identifier — the id prefix for routing, not the display name.
    pub fn identifier(&self) -> &str {
        match self {
            LibraryConfig::Offline { id, .. } => id,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Lowercase; any run of chars outside `[a-z0-9]` collapses to a single `-`;
/// trimmed; empty → `unknown`. Used by the create form's auto-identifier mode.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_sep = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            pending_sep = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_sep = true;
        }
    }
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}

/// [`slugify`] with `-2`/`-3`/… appended until the result is not in `taken`.
pub fn slugify_unique(name: &str, taken: &HashSet<String>) -> String {
    let base = slugify(name);
    if !taken.contains(&base) {
        return base;
    }
    for n in 2u32.. {
        let candidate = format!("{base}-{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

pub fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_collapses_and_trims() {
        assert_eq!(slugify("My Music"), "my-music");
        assert_eq!(slugify("  Foo:Bar  Baz "), "foo-bar-baz");
        assert_eq!(slugify("UPPER"), "upper");
        assert_eq!(slugify("a---b"), "a-b");
    }

    #[test]
    fn slugify_empty_becomes_unknown() {
        assert_eq!(slugify(""), "unknown");
        assert_eq!(slugify("   "), "unknown");
        assert_eq!(slugify("!!!"), "unknown");
        assert_eq!(slugify("Ünicode"), "nicode");
    }

    #[test]
    fn slugify_unique_appends_collision_suffixes() {
        let taken = HashSet::from(["my-music".to_string()]);
        assert_eq!(slugify_unique("My Music", &taken), "my-music-2");
        let taken = HashSet::from(["my-music".to_string(), "my-music-2".to_string()]);
        assert_eq!(slugify_unique("My Music", &taken), "my-music-3");
        assert_eq!(slugify_unique("Other", &taken), "other");
    }
}
