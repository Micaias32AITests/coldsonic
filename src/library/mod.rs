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
use std::fmt;
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
    /// An artist-level picture when the provider has one; offline libraries
    /// have none (their covers live on albums), so the table falls back to
    /// a placeholder icon.
    pub cover: Option<CoverId>,
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

    pub async fn track_bytes(&self, id: &TrackId) -> Result<Bytes, LibraryError> {
        match self {
            Library::Offline(library) => library.track_bytes(id).await,
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

impl fmt::Display for ScanMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tags => f.write_str("By Song Metadata"),
            Self::Folders => f.write_str("By Directory Structure"),
        }
    }
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
        /// Artist names that [`primary_artist`] must never split, e.g.
        /// `"For KING & COUNTRY"` or a literal `"Simpson, Lisa"`.
        /// Compared case-insensitively.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        never_split: Vec<String>,
    },
}

impl LibraryConfig {
    /// The stable identifier — the id prefix for routing, not the display name.
    pub fn identifier(&self) -> &str {
        match self {
            LibraryConfig::Offline { id, .. } => id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            LibraryConfig::Offline { name, .. } => name,
        }
    }

    pub fn enabled(&self) -> bool {
        match self {
            LibraryConfig::Offline { enabled, .. } => *enabled,
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

/// The primary artist of a raw credit string: everything before the first
/// feat-token (`feat.`, `ft.`, `featuring`, …) or collaboration separator
/// (`&`, `,`). Tags like `"Kendrick Lamar feat. GLC"` collapse to
/// `"Kendrick Lamar"` so the track groups under the primary artist
/// (MusicBrainz featuring style).
///
/// feat-stripping always runs (the guest is never the primary); the
/// `never_split` blacklist then protects the remaining head from `&`/`,`
/// splitting, for names that contain a separator as part of the artist
/// (e.g. `"For KING & COUNTRY"`). Entries match case-insensitively.
pub fn primary_artist(raw: &str, never_split: &[String]) -> String {
    let head = split_at_feat(raw.trim()).trim();
    if never_split
        .iter()
        .any(|keep| keep.trim().eq_ignore_ascii_case(head))
    {
        return head.to_string();
    }
    let primary = split_at_collab(head);
    let primary = primary.trim();
    if primary.is_empty() {
        raw.trim().to_string()
    } else {
        primary.to_string()
    }
}

/// Everything before the first collaboration separator: `" & "` or `", "`,
/// whichever comes first. Both are space-delimited, so `"A&B"`/`"A,B"` pass
/// through unchanged.
fn split_at_collab(s: &str) -> &str {
    let cut = [" & ", ", "].iter().filter_map(|sep| s.find(sep)).min();
    cut.map_or(s, |pos| &s[..pos])
}

/// Everything before the first feat-token (space-delimited, dot optional).
fn split_at_feat(s: &str) -> &str {
    let lower = s.to_ascii_lowercase();
    for needle in [" featuring ", " feat. ", " feat ", " ft. ", " ft "] {
        if let Some(pos) = lower.find(needle) {
            return &s[..pos];
        }
    }
    s
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

    #[test]
    fn primary_artist_splits_feat_tokens() {
        let none = [];
        assert_eq!(
            primary_artist("Kendrick Lamar feat. GLC", &none),
            "Kendrick Lamar"
        );
        assert_eq!(
            primary_artist("Kendrick Lamar feat GLC", &none),
            "Kendrick Lamar"
        );
        assert_eq!(primary_artist("A ft. B", &none), "A");
        assert_eq!(primary_artist("A ft B", &none), "A");
        assert_eq!(primary_artist("A Featuring B", &none), "A");
        assert_eq!(primary_artist("A FEAT. B", &none), "A");
    }

    #[test]
    fn primary_artist_splits_collab_separators() {
        let none = [];
        assert_eq!(
            primary_artist("Kendrick Lamar & SZA", &none),
            "Kendrick Lamar"
        );
        assert_eq!(
            primary_artist("Kendrick Lamar, SZA", &none),
            "Kendrick Lamar"
        );
        assert_eq!(primary_artist("A & B & C", &none), "A");
        assert_eq!(primary_artist("A, B, C", &none), "A");
        assert_eq!(primary_artist("A & B, C", &none), "A");
        assert_eq!(primary_artist("A, B & C", &none), "A");
        assert_eq!(primary_artist("A&B", &none), "A&B"); // needs spaces
        assert_eq!(primary_artist("A,B", &none), "A,B"); // needs spaces
    }

    #[test]
    fn primary_artist_never_split_short_circuits() {
        let keep = [
            "For KING & COUNTRY".to_string(),
            "Simon & Garfunkel".to_string(),
            "Simpson, Lisa".to_string(),
        ];
        assert_eq!(
            primary_artist("For KING & COUNTRY", &keep),
            "For KING & COUNTRY"
        );
        assert_eq!(
            primary_artist("  for king & country  ", &keep),
            "for king & country"
        );
        assert_eq!(
            primary_artist("Simon & Garfunkel feat. Someone", &keep),
            "Simon & Garfunkel"
        );
        assert_eq!(primary_artist("Simpson, Lisa", &keep), "Simpson, Lisa");
        assert_eq!(
            primary_artist("Kendrick Lamar & SZA", &keep),
            "Kendrick Lamar"
        );
        assert_eq!(
            primary_artist("Kendrick Lamar, SZA", &keep),
            "Kendrick Lamar"
        );
    }

    #[test]
    fn primary_artist_trims_and_falls_back() {
        let none = [];
        assert_eq!(primary_artist("  Solo  ", &none), "Solo");
        assert_eq!(primary_artist("feat. Only", &none), "feat. Only");
        assert_eq!(primary_artist("", &none), "");
        assert_eq!(primary_artist("A & B feat. C", &none), "A");
        assert_eq!(primary_artist("A feat. B & C", &none), "A");
        assert_eq!(primary_artist("A, B feat. C", &none), "A");
        assert_eq!(primary_artist("A feat. B, C", &none), "A");
    }
}
