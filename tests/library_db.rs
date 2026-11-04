//! Integration tests for the library databases, scans, and global data.
//!
//! Every test uses `Manager::open_at` with a temp data/cache dir, so the real
//! user directories are never touched.

use std::path::{Path, PathBuf};

use coldsonic::library::db::library::LibraryDb;
use coldsonic::library::{Album, LibraryConfig, Manager, ScanMode, Track, scan};
use tempfile::TempDir;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/music")
}

fn offline(name: &str, id: &str, root: PathBuf, scan_mode: ScanMode) -> LibraryConfig {
    LibraryConfig::Offline {
        name: name.into(),
        id: id.into(),
        root,
        scan: scan_mode,
        enabled: true,
        never_split: Vec::new(),
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

async fn open_at(configs: &[LibraryConfig], data: &Path, cache: &Path) -> Manager {
    Manager::open_at(configs, data, cache)
        .await
        .expect("manager open")
}

async fn scan_all(manager: &Manager, force: bool) {
    for plan in manager.scan_plan() {
        let outcome = scan::library(plan, force).await;
        outcome.result.expect("scan");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("entry");
        if entry.path().is_dir() {
            out.extend(walk(&entry.path()));
        } else {
            out.push(entry.path());
        }
    }
    out
}

#[tokio::test]
async fn first_scan_populates_rows_and_reopen_reads_without_scanning() {
    let temp = TempDir::new().expect("temp");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline(
        "Folders",
        "folders",
        fixtures().join("folders"),
        ScanMode::Folders,
    )];

    let manager = open_at(&configs, &data, &cache).await;
    let before = manager.artists().await.expect("pre-scan artists");
    assert!(before.is_empty(), "opening must not scan");

    scan_all(&manager, false).await;

    let artists = manager.artists().await.expect("artists");
    let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Alpha Artist", "Beta Artist", "Unknown Artist"]);
    assert!(
        artists
            .iter()
            .all(|a| a.id.as_str().starts_with("folders:"))
    );

    let albums = manager.albums().await.expect("albums");
    assert_eq!(albums.len(), 4);
    assert!(
        albums
            .iter()
            .all(|a| a.song_count == 1 || a.song_count == 2)
    );

    let tracks = manager.tracks().await.expect("tracks");
    assert_eq!(tracks.len(), 5);

    // Reopening the same database must not require a scan; the stat walk is
    // clean and the rows are identical.
    drop(manager);
    let reopened = open_at(&configs, &data, &cache).await;
    assert_eq!(reopened.artists().await.expect("artists"), artists);
    assert_eq!(reopened.albums().await.expect("albums"), albums);
    assert_eq!(reopened.tracks().await.expect("tracks"), tracks);

    scan_all(&reopened, false).await;
    assert_eq!(reopened.tracks().await.expect("tracks"), tracks);
}

#[tokio::test]
async fn stat_walk_detects_added_removed_and_modified_files() {
    let temp = TempDir::new().expect("temp");
    let root = temp.path().join("music");
    copy_tree(&fixtures().join("folders"), &root);
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline(
        "Folders",
        "folders",
        root.clone(),
        ScanMode::Folders,
    )];

    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;

    let db = LibraryDb::open(&coldsonic::library::db::library_db_path(&data, "folders"))
        .await
        .expect("db");
    let stored = db.files().await.expect("files");
    assert_eq!(
        scan::check(&root, &stored).expect("check"),
        scan::ScanCheck::Clean
    );

    // Added.
    let added = root.join("Alpha Artist/Album One/03 - Added.mp3");
    std::fs::copy(fixtures().join("stub.mp3"), &added).expect("copy added");
    assert!(matches!(
        scan::check(&root, &stored).expect("check"),
        scan::ScanCheck::NeedsScan(_)
    ));

    // Modified.
    let file = root.join("Alpha Artist/Album One/01 - First.mp3");
    std::fs::write(&file, b"changed").expect("modify");
    assert!(matches!(
        scan::check(&root, &stored).expect("check"),
        scan::ScanCheck::NeedsScan(_)
    ));
    std::fs::remove_file(&added).expect("remove added");
    assert!(matches!(
        scan::check(&root, &stored).expect("check"),
        scan::ScanCheck::NeedsScan(_)
    ));

    // Removed.
    std::fs::remove_file(&file).expect("remove");
    let stored = db.files().await.expect("files");
    assert!(matches!(
        scan::check(&root, &stored).expect("check"),
        scan::ScanCheck::NeedsScan(_)
    ));
}

#[tokio::test]
async fn rescan_replaces_rows_without_orphans() {
    let temp = TempDir::new().expect("temp");
    let root = temp.path().join("music");
    copy_tree(&fixtures().join("folders"), &root);
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline(
        "Folders",
        "folders",
        root.clone(),
        ScanMode::Folders,
    )];

    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;
    assert_eq!(manager.tracks().await.expect("tracks").len(), 5);

    std::fs::remove_dir_all(root.join("Beta Artist")).expect("remove beta");
    scan_all(&manager, true).await;

    let tracks = manager.tracks().await.expect("tracks");
    assert_eq!(tracks.len(), 4);
    assert!(
        tracks
            .iter()
            .all(|track| !track.id.as_str().contains("Beta"))
    );
    let albums = manager.albums().await.expect("albums");
    assert_eq!(albums.len(), 3);
    assert!(
        albums
            .iter()
            .all(|album: &Album| !album.name.contains("Three"))
    );

    // No file rows point at missing tracks.
    let db = LibraryDb::open(&coldsonic::library::db::library_db_path(&data, "folders"))
        .await
        .expect("db");
    let files = db.files().await.expect("files");
    assert_eq!(files.len(), walk(&root).len());
}

#[tokio::test]
async fn scanner_version_change_invalidates_the_completed_scan() {
    let temp = TempDir::new().expect("temp");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline(
        "Folders",
        "folders",
        fixtures().join("folders"),
        ScanMode::Folders,
    )];
    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;

    let path = coldsonic::library::db::library_db_path(&data, "folders");
    let db = LibraryDb::open(&path).await.expect("db");
    sqlx::query("UPDATE meta SET value = 'older' WHERE key = 'scanner_version'")
        .execute(&db.pool)
        .await
        .expect("age scanner version");
    assert!(db.scan_complete().await.expect("complete"));

    // Reopening applies the signature, notices the version change, and forces
    // the next startup scan to run.
    drop(db);
    drop(manager);
    let manager = open_at(&configs, &data, &cache).await;
    let db = LibraryDb::open(&path).await.expect("db");
    assert!(!db.scan_complete().await.expect("complete"));
    drop(manager);
}

#[tokio::test]
async fn empty_library_is_not_an_error() {
    let temp = TempDir::new().expect("temp");
    let root = temp.path().join("empty");
    std::fs::create_dir_all(&root).expect("empty root");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline("Empty", "empty", root, ScanMode::Tags)];

    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;
    assert!(manager.artists().await.expect("artists").is_empty());
    assert!(manager.albums().await.expect("albums").is_empty());
    assert!(manager.tracks().await.expect("tracks").is_empty());
}

#[tokio::test]
async fn folder_art_reads_from_the_library_root() {
    let temp = TempDir::new().expect("temp");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline(
        "Folders",
        "folders",
        fixtures().join("folders"),
        ScanMode::Folders,
    )];
    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;

    let artist = manager
        .artists()
        .await
        .expect("artists")
        .into_iter()
        .find(|artist| artist.name == "Alpha Artist")
        .expect("alpha");
    let artist = manager.artist(&artist.id).await.expect("artist");
    let album = manager.album(&artist.albums[0].id).await.expect("album");
    let cover = album.album.cover.expect("folder cover");
    let bytes = manager.cover(&cover, 0).await.expect("cover bytes");
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn embedded_cover_is_cached_on_disk() {
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::{Accessor, TaggedFileExt};
    use lofty::tag::{Tag, TagExt, TagType};

    let temp = TempDir::new().expect("temp");
    let root = temp.path().join("tags");
    copy_tree(&fixtures().join("tags"), &root);
    let jpg = std::fs::read(fixtures().join("cover.jpg")).expect("fixture art");

    let path = root.join("gamma.mp3");
    let mut tagged = lofty::read_from_path(&path).expect("read tags");
    if tagged.primary_tag().is_none() && tagged.first_tag().is_none() {
        tagged.insert_tag(Tag::new(TagType::Id3v2));
    }
    let tag = if tagged.primary_tag().is_some() {
        tagged.primary_tag_mut().expect("primary")
    } else {
        tagged.first_tag_mut().expect("first")
    };
    tag.set_title("Gamma Song".to_string());
    tag.set_artist("Beta Artist".to_string());
    tag.set_album("Album Three".to_string());
    tag.set_track(1);
    tag.push_picture(
        Picture::unchecked(jpg.clone())
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Jpeg)
            .build(),
    );
    tag.save_to_path(&path, WriteOptions::default())
        .expect("save tags");

    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline("Tags", "tags", root.clone(), ScanMode::Tags)];
    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;

    let artist = manager
        .artists()
        .await
        .expect("artists")
        .into_iter()
        .find(|artist| artist.name == "Beta Artist")
        .expect("beta");
    let artist = manager.artist(&artist.id).await.expect("artist");
    let album = manager.album(&artist.albums[0].id).await.expect("album");
    let cover = album.album.cover.expect("embedded cover");
    assert_eq!(cover.as_str(), "tags:gamma.mp3");

    let covers_dir = coldsonic::library::db::library_cover_cache_dir(&cache, "tags");
    let entries: Vec<_> = std::fs::read_dir(&covers_dir)
        .expect("cover cache dir")
        .collect();
    assert_eq!(entries.len(), 1, "one cached blob");

    let bytes = manager.cover(&cover, 0).await.expect("cover bytes");
    assert_eq!(bytes.as_ref(), jpg.as_slice());
}

#[tokio::test]
async fn detail_ids_disappear_as_not_found() {
    let temp = TempDir::new().expect("temp");
    let root = temp.path().join("music");
    copy_tree(&fixtures().join("manager-a"), &root);
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline("Alpha", "alpha", root.clone(), ScanMode::Folders)];

    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;
    let album = manager
        .albums()
        .await
        .expect("albums")
        .into_iter()
        .find(|album| album.name == "Gamma Album")
        .expect("gamma");
    let album_id = album.id;
    manager.album(&album_id).await.expect("album exists");

    std::fs::remove_dir_all(root.join("Gamma Artist")).expect("remove");
    scan_all(&manager, true).await;
    let err = manager.album(&album_id).await.expect_err("gone");
    assert!(matches!(err, coldsonic::library::LibraryError::NotFound(_)));
}

#[tokio::test]
async fn global_playlists_and_history_survive_dangling_libraries() {
    let temp = TempDir::new().expect("temp");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configs = vec![offline(
        "Folders",
        "folders",
        fixtures().join("folders"),
        ScanMode::Folders,
    )];
    let manager = open_at(&configs, &data, &cache).await;
    scan_all(&manager, false).await;

    let track: Track = manager.tracks().await.expect("tracks").remove(0);
    let playlist = manager.create_playlist("Mix").await.expect("create");
    manager
        .add_playlist_track(&playlist.id, &track)
        .await
        .expect("add");
    let entries = manager
        .playlist_tracks(&playlist.id)
        .await
        .expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].track_id, track.id);
    assert_eq!(entries[0].title.as_deref(), Some(track.title.as_str()));

    manager
        .record_play(&track.id, 123, Some(456), true)
        .await
        .expect("history");
    let history = manager.play_history(10).await.expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].track_id, track.id);
    assert!(history[0].completed);

    // Reopen with no libraries: global rows remain readable and snapshots
    // render even though the track can no longer be resolved.
    drop(manager);
    let manager = open_at(&[], &data, &cache).await;
    let entries = manager
        .playlist_tracks(&playlist.id)
        .await
        .expect("dangling entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].track_id, track.id);
    assert!(manager.track_bytes(&entries[0].track_id).await.is_err());

    manager
        .rename_playlist(&playlist.id, "Renamed")
        .await
        .expect("rename");
    let playlists = manager.playlists().await.expect("playlists");
    assert_eq!(playlists[0].name, "Renamed");
    manager.delete_playlist(&playlist.id).await.expect("delete");
    assert!(manager.playlists().await.expect("playlists").is_empty());
}
