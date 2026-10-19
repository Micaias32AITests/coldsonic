//! Offline provider — local files, scanned once at open.
//!
//! [`OfflineLibrary::open`] runs the blocking filesystem walk + tag reads
//! (via [`scan`]) inside `smol::unblock`, producing a fully indexed
//! [`MemoryStore`]. Every later query is a cheap in-memory read behind the
//! same async signature. An empty music folder is not an error — open
//! succeeds with an empty index and the UI shows an empty state.

mod scan;
mod store;

use std::path::PathBuf;

use bytes::Bytes;

use crate::library::{
    AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, CoverId, LibraryError, ScanMode, TrackId,
};

use store::MemoryStore;

#[derive(Debug, Clone)]
pub struct OfflineLibrary {
    /// Kept for a future rescan; not read after open.
    #[allow(dead_code)]
    root: PathBuf,
    /// Kept for a future rescan; not read after open.
    #[allow(dead_code)]
    mode: ScanMode,
    /// Kept for a future rescan; not read after open.
    #[allow(dead_code)]
    never_split: Vec<String>,
    store: MemoryStore,
}

impl OfflineLibrary {
    pub async fn open(
        root: PathBuf,
        mode: ScanMode,
        never_split: Vec<String>,
    ) -> Result<Self, LibraryError> {
        let scan_root = root.clone();
        let scan_split = never_split.clone();
        let data = smol::unblock(move || scan::scan(&scan_root, mode, &scan_split)).await?;
        Ok(Self {
            root,
            mode,
            never_split,
            store: MemoryStore::from(data),
        })
    }

    pub async fn artists(&self) -> Result<Vec<Artist>, LibraryError> {
        Ok(self.store.artists().await)
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<ArtistDetail, LibraryError> {
        self.store
            .artist(id)
            .await
            .ok_or_else(|| LibraryError::NotFound(format!("no artist {}", id.as_str())))
    }

    pub async fn album(&self, id: &AlbumId) -> Result<AlbumDetail, LibraryError> {
        self.store
            .album(id)
            .await
            .ok_or_else(|| LibraryError::NotFound(format!("no album {}", id.as_str())))
    }

    pub async fn cover(&self, id: &CoverId, px: u32) -> Result<Bytes, LibraryError> {
        self.store
            .cover_bytes(id, px)
            .await
            .map(Bytes::from)
            .ok_or_else(|| LibraryError::NotFound(format!("no cover {}", id.as_str())))
    }

    pub async fn track_bytes(&self, id: &TrackId) -> Result<Bytes, LibraryError> {
        let rel = id.as_str();
        let path = self.root.join(rel);
        let data = smol::fs::read(&path)
            .await
            .map_err(LibraryError::Io)?;
        Ok(Bytes::from(data))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::{Accessor, TaggedFileExt};
    use lofty::tag::{Tag, TagExt, TagType};

    use super::*;
    use crate::library::{AlbumId, ArtistId, Library};

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/music")
    }

    /// Unique temp dir per test; cleaned up on drop.
    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "coldsonic-library-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).expect("create temp tree");
            Self { root }
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn copy_tree(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).expect("create tree dir");
        for entry in std::fs::read_dir(src).expect("read tree dir") {
            let entry = entry.expect("tree entry");
            let to = dst.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &to);
            } else {
                std::fs::copy(entry.path(), to).expect("copy tree file");
            }
        }
    }

    /// Tags-mode fixtures ship untagged; tests copy them to a temp dir and
    /// write tags into the copies.
    fn write_tags(
        path: &Path,
        title: Option<&str>,
        artist: Option<&str>,
        album: Option<&str>,
        track: Option<u32>,
        picture: Option<Vec<u8>>,
    ) {
        let mut tagged = lofty::read_from_path(path).expect("read tags");
        if tagged.primary_tag().is_none() && tagged.first_tag().is_none() {
            tagged.insert_tag(Tag::new(TagType::Id3v2));
        }
        let has_primary = tagged.primary_tag().is_some();
        let tag = if has_primary {
            tagged.primary_tag_mut().expect("primary tag")
        } else {
            tagged.first_tag_mut().expect("first tag")
        };
        if let Some(value) = title {
            tag.set_title(value.to_string());
        }
        if let Some(value) = artist {
            tag.set_artist(value.to_string());
        }
        if let Some(value) = album {
            tag.set_album(value.to_string());
        }
        if let Some(value) = track {
            tag.set_track(value);
        }
        if let Some(data) = picture {
            tag.push_picture(
                Picture::unchecked(data)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Jpeg)
                    .build(),
            );
        }
        tag.save_to_path(path, WriteOptions::default())
            .expect("save tags");
    }

    fn open(root: PathBuf, mode: ScanMode) -> OfflineLibrary {
        smol::block_on(OfflineLibrary::open(root, mode, Vec::new())).expect("open library")
    }

    fn open_with_never_split(
        root: PathBuf,
        mode: ScanMode,
        never_split: Vec<String>,
    ) -> OfflineLibrary {
        smol::block_on(OfflineLibrary::open(root, mode, never_split)).expect("open library")
    }

    #[test]
    fn tags_mode_groups_orders_and_finds_art() {
        let temp = TempTree::new();
        copy_tree(&fixtures().join("tags"), temp.path());
        let jpg = std::fs::read(fixtures().join("cover.jpg")).expect("fixture art");
        write_tags(
            &temp.path().join("alpha.mp3"),
            Some("Alpha Song"),
            Some("Alpha Artist"),
            Some("Album One"),
            Some(2),
            None,
        );
        write_tags(
            &temp.path().join("beta.mp3"),
            Some("Beta Song"),
            Some("Alpha Artist"),
            Some("Album One"),
            Some(1),
            None,
        );
        write_tags(
            &temp.path().join("gamma.mp3"),
            Some("Gamma Song"),
            Some("Beta Artist"),
            Some("Album Three"),
            Some(1),
            Some(jpg),
        );
        // delta.mp3 stays untagged: unknown-artist fallback.

        let lib = open(temp.path().to_path_buf(), ScanMode::Tags);

        let artists = smol::block_on(lib.artists()).expect("artists");
        let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Alpha Artist", "Beta Artist", "Unknown Artist"]);
        assert!(artists.iter().all(|a| a.album_count == 1));

        let alpha = smol::block_on(lib.artist(&ArtistId::new("Alpha Artist"))).expect("alpha");
        assert_eq!(alpha.albums.len(), 1);
        assert_eq!(alpha.albums[0].name, "Album One");
        assert_eq!(alpha.albums[0].song_count, 2);

        let album_id = AlbumId::new("Alpha Artist\u{0}Album One");
        let album = smol::block_on(lib.album(&album_id)).expect("album one");
        let titles: Vec<&str> = album.songs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Beta Song", "Alpha Song"]);
        assert!(album.songs.iter().all(|t| t.duration > Duration::ZERO));
        assert!(album.songs.iter().all(|t| t.track_no.is_some()));

        let beta = smol::block_on(lib.artist(&ArtistId::new("Beta Artist"))).expect("beta");
        let three = smol::block_on(lib.album(&beta.albums[0].id.clone())).expect("album three");
        let cover = three.album.cover.expect("embedded art becomes album cover");
        assert_eq!(cover.as_str(), "gamma.mp3");
        assert!(three.songs.iter().all(|t| t.cover.as_ref() == Some(&cover)));
        let bytes = smol::block_on(lib.cover(&cover, 0)).expect("cover bytes");
        assert!(!bytes.is_empty());

        let unknown =
            smol::block_on(lib.artist(&ArtistId::new("Unknown Artist"))).expect("unknown");
        assert_eq!(unknown.albums[0].name, "Unknown Album");
        let loose =
            smol::block_on(lib.album(&unknown.albums[0].id.clone())).expect("unknown album");
        assert_eq!(loose.songs[0].title, "delta");
        assert_eq!(loose.songs[0].artist.as_deref(), Some("Unknown Artist"));
    }

    #[test]
    fn folders_mode_parses_paths_and_folder_art() {
        let lib = open(fixtures().join("folders"), ScanMode::Folders);

        let artists = smol::block_on(lib.artists()).expect("artists");
        let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Alpha Artist", "Beta Artist", "Unknown Artist"]);
        assert_eq!(artists[0].album_count, 2);

        let alpha = smol::block_on(lib.artist(&ArtistId::new("Alpha Artist"))).expect("alpha");
        let album_names: Vec<&str> = alpha.albums.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(album_names, ["Album One", "Album Two"]);

        let one = smol::block_on(lib.album(&alpha.albums[0].id.clone())).expect("album one");
        assert_eq!(one.songs[0].title, "First");
        assert_eq!(one.songs[0].track_no, Some(1));
        assert_eq!(one.songs[1].title, "Second");
        assert!(one.songs.iter().all(|t| t.duration > Duration::ZERO));
        let cover = one.album.cover.expect("cover.jpg next to the album");
        assert_eq!(cover.as_str(), "Alpha Artist/Album One");
        let bytes = smol::block_on(lib.cover(&cover, 0)).expect("folder art bytes");
        assert!(!bytes.is_empty());

        let beta = smol::block_on(lib.artist(&ArtistId::new("Beta Artist"))).expect("beta");
        let three = smol::block_on(lib.album(&beta.albums[0].id.clone())).expect("album three");
        assert_eq!(
            three.album.cover.as_ref().map(|c| c.as_str()),
            Some("Beta Artist/Album Three")
        );

        let unknown =
            smol::block_on(lib.artist(&ArtistId::new("Unknown Artist"))).expect("unknown");
        let loose =
            smol::block_on(lib.album(&unknown.albums[0].id.clone())).expect("unknown album");
        assert_eq!(loose.album.name, "Unknown Album");
        assert_eq!(loose.songs[0].title, "loose");
        assert_eq!(loose.songs[0].track_no, None);
        assert_eq!(loose.songs[0].artist.as_deref(), Some("Unknown Artist"));
    }

    #[test]
    fn ids_stable_across_rescans() {
        let root = fixtures().join("folders");
        let first = open(root.clone(), ScanMode::Folders);
        let second = open(root, ScanMode::Folders);

        let a1 = smol::block_on(first.artists()).expect("first artists");
        let a2 = smol::block_on(second.artists()).expect("second artists");
        assert_eq!(a1, a2);

        let d1 = smol::block_on(first.artist(&a1[0].id.clone())).expect("first detail");
        let d2 = smol::block_on(second.artist(&a2[0].id.clone())).expect("second detail");
        assert_eq!(d1.albums, d2.albums);

        let s1 = smol::block_on(first.album(&d1.albums[0].id.clone())).expect("first songs");
        let s2 = smol::block_on(second.album(&d2.albums[0].id.clone())).expect("second songs");
        assert_eq!(s1.songs, s2.songs);
    }

    #[test]
    fn empty_tree_opens_with_empty_index() {
        let lib = open(fixtures().join("empty"), ScanMode::Tags);
        let artists = smol::block_on(lib.artists()).expect("artists");
        assert!(artists.is_empty());
    }

    #[test]
    fn missing_root_is_an_error() {
        let err = smol::block_on(OfflineLibrary::open(
            PathBuf::from("/definitely/not/a/real/library/root"),
            ScanMode::Tags,
            Vec::new(),
        ))
        .expect_err("missing root");
        assert!(matches!(err, LibraryError::Io(_)));
    }

    #[test]
    fn library_enum_delegates_to_offline() {
        let offline = open(fixtures().join("folders"), ScanMode::Folders);
        let library = Library::Offline(offline);

        let artists = smol::block_on(library.artists()).expect("artists");
        assert!(!artists.is_empty());
        let alpha = smol::block_on(library.artist(&artists[0].id.clone())).expect("artist");
        let album = smol::block_on(library.album(&alpha.albums[0].id.clone())).expect("album");
        let cover = album.album.cover.expect("cover through the enum");
        let bytes = smol::block_on(library.cover(&cover, 0)).expect("cover bytes");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn tags_mode_normalizes_primary_artist() {
        let temp = TempTree::new();
        copy_tree(&fixtures().join("tags"), temp.path());
        write_tags(
            &temp.path().join("alpha.mp3"),
            Some("Alpha Song"),
            Some("Alpha Artist feat. Guest"),
            Some("Album One"),
            Some(1),
            None,
        );
        write_tags(
            &temp.path().join("beta.mp3"),
            Some("Beta Song"),
            Some("Alpha Artist & Friend"),
            Some("Album One"),
            Some(2),
            None,
        );
        write_tags(
            &temp.path().join("gamma.mp3"),
            Some("Gamma Song"),
            Some("Alpha Artist, Guest Two"),
            Some("Album Two"),
            Some(1),
            None,
        );
        // delta.mp3 stays untagged: unknown-artist fallback.

        let lib = open(temp.path().to_path_buf(), ScanMode::Tags);

        let artists = smol::block_on(lib.artists()).expect("artists");
        let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Alpha Artist", "Unknown Artist"]);
        let alpha = smol::block_on(lib.artist(&ArtistId::new("Alpha Artist"))).expect("alpha");
        assert_eq!(alpha.albums.len(), 2);
        assert_eq!(alpha.albums[0].song_count, 2);
        assert_eq!(alpha.albums[1].song_count, 1);
    }

    #[test]
    fn tags_mode_never_split_blacklist() {
        let temp = TempTree::new();
        copy_tree(&fixtures().join("tags"), temp.path());
        write_tags(
            &temp.path().join("alpha.mp3"),
            Some("Alpha Song"),
            Some("For KING & COUNTRY"),
            Some("Burn the Ships"),
            Some(1),
            None,
        );
        write_tags(
            &temp.path().join("beta.mp3"),
            Some("Beta Song"),
            Some("Simon & Garfunkel"),
            Some("Bridge Over Troubled Water"),
            Some(1),
            None,
        );

        let lib = open_with_never_split(
            temp.path().to_path_buf(),
            ScanMode::Tags,
            vec!["For KING & COUNTRY".into(), "simon & garfunkel".into()],
        );

        let artists = smol::block_on(lib.artists()).expect("artists");
        let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
        // gamma/delta.mp3 stay untagged: unknown-artist fallback.
        assert_eq!(
            names,
            ["For KING & COUNTRY", "Simon & Garfunkel", "Unknown Artist"]
        );
    }

    #[test]
    fn folders_mode_trims_but_never_splits() {
        let temp = TempTree::new();
        let album_dir = temp
            .path()
            .join("For KING & COUNTRY")
            .join("Burn the Ships");
        std::fs::create_dir_all(&album_dir).expect("create album dir");
        std::fs::copy(
            fixtures().join("tags").join("alpha.mp3"),
            album_dir.join("01 - Track.mp3"),
        )
        .expect("copy track");

        let lib = open(temp.path().to_path_buf(), ScanMode::Folders);

        let artists = smol::block_on(lib.artists()).expect("artists");
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "For KING & COUNTRY");
    }
}
