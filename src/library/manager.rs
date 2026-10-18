//! Manager — the only library seam the UI touches.
//!
//! Screens hold an `Arc<Manager>` and ask it about artists/albums/tracks/
//! covers; providers never reach the screens, and nothing above the manager
//! knows how many libraries exist or how they store data. The manager opens
//! every enabled config up front, routes prefixed ids back down by pure
//! string work, re-prefixes everything on the way out, and owns the single
//! cache seam where sqlite will later slot in — the merge policy lives only
//! here.

use std::collections::HashSet;

use bytes::Bytes;

use crate::library::is_valid_id;

use super::offline::OfflineLibrary;
use super::{
    Album, AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, CoverId, Library, LibraryConfig,
    LibraryError, Track, TrackId,
};

#[derive(Debug)]
pub struct Manager {
    /// Enabled entries only. Looked up by `LibraryConfig` identifier;
    /// that identifier is the id prefix. Vec, not map — order documents
    /// config order for debugging, and lookups are by string anyway.
    libraries: Vec<Opened>,
    /// Never read today; sqlite slots in here later.
    #[allow(dead_code)]
    cache: CacheStore,
}

#[derive(Debug)]
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
}

impl Manager {
    /// Opens every enabled library, in order. Duplicate or malformed
    /// identifiers are rejected before any scanning starts; an enabled entry
    /// that fails to open fails the whole `open` with an error naming it —
    /// a typo'd root must be loud, not a silently smaller library.
    pub async fn open(library_configs: &[LibraryConfig]) -> Result<Manager, LibraryError> {
        let mut seen = HashSet::new();
        for config in library_configs {
            let id = config.identifier();
            if !is_valid_id(id) {
                return Err(LibraryError::Invalid(format!(
                    "invalid library identifier '{id}': expected [a-z0-9-]+"
                )));
            }
            if !seen.insert(id) {
                return Err(LibraryError::Invalid(format!(
                    "duplicate library identifier '{id}'"
                )));
            }
        }

        let mut libraries = Vec::new();
        for config in library_configs {
            let LibraryConfig::Offline {
                name,
                id,
                root,
                scan,
                enabled,
            } = config;
            if !*enabled {
                continue;
            }
            let library = OfflineLibrary::open(root.clone(), *scan)
                .await
                .map_err(|err| {
                    LibraryError::Invalid(format!(
                        "failed to open library '{name}' (id '{id}'): {err}"
                    ))
                })?;
            libraries.push(Opened {
                config: config.clone(),
                library: Library::Offline(library),
            });
        }
        Ok(Manager {
            libraries,
            cache: CacheStore::None,
        })
    }

    /// Concatenates every library's artists, prefixes each id, and sorts by
    /// name case-insensitively (stable for ties). Same-named artists from
    /// different libraries stay separate rows — merging them is a
    /// display-policy question for later, and this is the only place it can
    /// change.
    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        let mut artists = Vec::new();
        for opened in &self.libraries {
            let prefix = opened.config.identifier();
            for mut artist in opened.library.artists().await? {
                artist.id = ArtistId::new(format!("{prefix}:{}", artist.id.as_str()));
                artists.push(artist);
            }
        }
        artists.sort_by_key(|a| a.name.to_lowercase());
        Ok(artists)
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        let (opened, rest) = route(&self.libraries, id.as_str())?;
        let mut detail = opened.library.artist(&ArtistId::new(rest)).await?;
        let prefix = opened.config.identifier();
        detail.artist.id = ArtistId::new(format!("{prefix}:{}", detail.artist.id.as_str()));
        for album in &mut detail.albums {
            re_prefix_album(prefix, album);
        }
        Ok(detail)
    }

    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        let (opened, rest) = route(&self.libraries, id.as_str())?;
        let mut detail = opened.library.album(&AlbumId::new(rest)).await?;
        let prefix = opened.config.identifier();
        re_prefix_album(prefix, &mut detail.album);
        for song in &mut detail.songs {
            re_prefix_track(prefix, song);
        }
        Ok(detail)
    }

    pub async fn cover(&self, id: &CoverId, px: u32) -> Result<Bytes, LibraryError> {
        let (opened, rest) = route(&self.libraries, id.as_str())?;
        opened.library.cover(&CoverId::new(rest), px).await
    }
}

/// Split at the first `:`, find the library whose identifier matches, strip
/// the prefix. No provider is ever asked "is this yours?".
fn route<'a>(libraries: &'a [Opened], id: &str) -> Result<(&'a Opened, String), LibraryError> {
    let Some((prefix, rest)) = id.split_once(':') else {
        return Err(LibraryError::NotFound(format!(
            "unprefixed library id: {id}"
        )));
    };
    let opened = libraries
        .iter()
        .find(|o| o.config.identifier() == prefix)
        .ok_or_else(|| LibraryError::NotFound(format!("unknown library identifier: {prefix}")))?;
    Ok((opened, rest.to_string()))
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::library::ScanMode;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/music")
            .join(name)
    }

    fn offline(
        name: &str,
        id: &str,
        root: PathBuf,
        enabled: bool,
        scan: ScanMode,
    ) -> LibraryConfig {
        LibraryConfig::Offline {
            name: name.into(),
            id: id.into(),
            root,
            scan,
            enabled,
        }
    }

    fn open(configs: &[LibraryConfig]) -> Manager {
        smol::block_on(Manager::open(configs)).expect("manager open")
    }

    #[test]
    fn open_skips_disabled_entries() {
        let configs = vec![
            offline(
                "Alpha",
                "alpha",
                fixture("folders"),
                true,
                ScanMode::Folders,
            ),
            // Points nowhere: opening it would fail the whole Manager::open.
            offline(
                "Ghost",
                "ghost",
                PathBuf::from("/definitely/missing/library"),
                false,
                ScanMode::Tags,
            ),
        ];
        let manager = open(&configs);
        let artists = smol::block_on(manager.artists()).expect("artists");
        assert!(!artists.is_empty());
        assert!(artists.iter().all(|a| a.id.as_str().starts_with("alpha:")));
    }

    #[test]
    fn artists_merge_sorted_with_prefixed_ids() {
        let configs = vec![
            offline(
                "Alpha",
                "alpha",
                fixture("manager-a"),
                true,
                ScanMode::Folders,
            ),
            offline(
                "Beta",
                "beta",
                fixture("manager-b"),
                true,
                ScanMode::Folders,
            ),
        ];
        let manager = open(&configs);
        let artists = smol::block_on(manager.artists()).expect("artists");
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].name, "Delta Artist");
        assert_eq!(artists[0].id.as_str(), "beta:Delta Artist");
        assert_eq!(artists[1].name, "Gamma Artist");
        assert_eq!(artists[1].id.as_str(), "alpha:Gamma Artist");
    }

    #[test]
    fn album_and_cover_route_by_identifier_and_reprefix() {
        let configs = vec![offline(
            "Alpha",
            "alpha",
            fixture("manager-a"),
            true,
            ScanMode::Folders,
        )];
        let manager = open(&configs);

        let album_id = AlbumId::new("alpha:Gamma Artist\u{0}Gamma Album");
        let detail = smol::block_on(manager.album(&album_id)).expect("album");
        assert_eq!(
            detail.album.id.as_str(),
            "alpha:Gamma Artist\u{0}Gamma Album"
        );
        assert_eq!(
            detail.songs[0].id.as_str(),
            "alpha:Gamma Artist/Gamma Album/01 - One.mp3"
        );
        let cover = detail.songs[0]
            .cover
            .clone()
            .expect("folder art on the track");
        assert_eq!(cover.as_str(), "alpha:Gamma Artist/Gamma Album");
        assert_eq!(detail.album.cover.as_ref(), Some(&cover));
        let bytes = smol::block_on(manager.cover(&cover, 0)).expect("cover bytes");
        assert!(!bytes.is_empty());

        let artist_id = ArtistId::new("alpha:Gamma Artist");
        let artist = smol::block_on(manager.artist(&artist_id)).expect("artist");
        assert_eq!(artist.artist.id.as_str(), "alpha:Gamma Artist");
        assert_eq!(
            artist.albums[0].id.as_str(),
            "alpha:Gamma Artist\u{0}Gamma Album"
        );
    }

    #[test]
    fn unprefixed_and_unknown_ids_are_not_found() {
        let configs = vec![offline(
            "Alpha",
            "alpha",
            fixture("manager-a"),
            true,
            ScanMode::Folders,
        )];
        let manager = open(&configs);

        let unprefixed = AlbumId::new("Gamma Artist\u{0}Gamma Album");
        let err = smol::block_on(manager.album(&unprefixed)).expect_err("unprefixed");
        assert!(matches!(err, LibraryError::NotFound(_)));

        let unknown = AlbumId::new("zzz:whatever");
        let err = smol::block_on(manager.album(&unknown)).expect_err("unknown prefix");
        assert!(matches!(err, LibraryError::NotFound(_)));
    }

    #[test]
    fn duplicate_and_malformed_identifiers_reject_open() {
        let duplicate = vec![
            offline("A", "same", fixture("manager-a"), true, ScanMode::Folders),
            // Validation covers every entry, enabled or not.
            offline("B", "same", fixture("manager-b"), false, ScanMode::Tags),
        ];
        let err = smol::block_on(Manager::open(&duplicate)).expect_err("duplicate id");
        assert!(matches!(err, LibraryError::Invalid(_)));

        let malformed = vec![offline(
            "A",
            "Bad Id",
            fixture("manager-a"),
            true,
            ScanMode::Folders,
        )];
        let err = smol::block_on(Manager::open(&malformed)).expect_err("malformed id");
        assert!(matches!(err, LibraryError::Invalid(_)));
    }

    #[test]
    fn open_failure_names_the_offending_library() {
        let configs = vec![offline(
            "Broken",
            "broken",
            PathBuf::from("/definitely/missing/library"),
            true,
            ScanMode::Tags,
        )];
        let err = smol::block_on(Manager::open(&configs)).expect_err("missing root");
        match err {
            LibraryError::Invalid(msg) => {
                assert!(msg.contains("Broken"));
                assert!(msg.contains("broken"));
            }
            other => panic!("expected Invalid naming the library, got {other:?}"),
        }
    }
}
