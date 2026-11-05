//! Filesystem walk, tag reads, and the scan pipeline.
//!
//! The walk and the lofty calls are blocking; they run inside
//! `tokio::task::spawn_blocking`. The database write happens afterwards in one
//! short transaction, so readers continue to see the previous WAL snapshot
//! until the scan commits.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};
use lofty::tag::{ItemKey, Tag};
use tracing::info;

use crate::library::db::library::{CoverKind, FileStat, LibraryDb, ScanSignature};
use crate::library::identity::{self, RawTrack, ResolvedAlbum, UNKNOWN_ALBUM, UNKNOWN_ARTIST};
use crate::library::manager::{LibraryHandle, ScanOutcome, ScanPlan};
use crate::library::{LibraryConfig, ScanMode};

const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "flac", "ogg", "opus", "m4a", "wav"];

const FOLDER_ART_NAMES: [&str; 4] = ["cover.jpg", "cover.png", "folder.jpg", "folder.png"];

/// Everything a scan produces. No database handle, no UI types.
#[derive(Debug, Clone)]
pub struct ScanData {
    pub artists: Vec<ScanArtist>,
    pub albums: Vec<ScanAlbum>,
    pub tracks: Vec<ScanTrack>,
    pub covers: Vec<ScanCover>,
    pub files: Vec<ScanFile>,
}

#[derive(Debug, Clone)]
pub struct ScanArtist {
    pub id: String,
    pub name: String,
    pub sort_name: Option<String>,
    pub mb_artist_id: Option<String>,
    pub album_count: u32,
    pub cover: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScanAlbum {
    pub id: String,
    pub name: String,
    pub artist_id: String,
    /// Display credit, kept whole (`"Alice & Bob"`, `"Various Artists"`, …).
    pub artist: String,
    pub album_sort_name: Option<String>,
    pub compilation: bool,
    pub mb_release_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub disc_count: Option<u32>,
    pub song_count: u32,
    pub duration: Option<Duration>,
    pub cover: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScanTrack {
    pub id: String,
    pub title: String,
    pub artist_id: String,
    /// Full `ARTIST` credit, never collapsed to the primary artist.
    pub artist: String,
    pub album_id: String,
    pub album: String,
    pub track_no: Option<u32>,
    pub duration: Duration,
    pub cover: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScanCover {
    pub id: String,
    pub kind: CoverKind,
    pub source_path: String,
    pub mime: Option<String>,
    pub mtime_ns: i64,
    pub size: i64,
    /// `Some` only for embedded art.
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanKind {
    Audio,
    Cover,
}

impl ScanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Cover => "cover",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanFile {
    pub path: String,
    pub mtime_ns: i64,
    pub size: i64,
    pub kind: ScanKind,
    pub track_id: Option<String>,
}

/// A discovered file, before any tag read.
struct WalkedFile {
    rel: String,
    path: PathBuf,
    mtime_ns: i64,
    size: i64,
    kind: ScanKind,
}

/// A parsed audio file: identity fields plus everything the scan needs to
/// build rows and covers.
struct FileMeta {
    rel: String,
    raw: RawTrack,
    duration: Duration,
    mtime_ns: i64,
    size: i64,
    embedded_art: Option<(Vec<u8>, Option<String>)>,
}

#[derive(Clone)]
struct CoverSource {
    id: String,
    kind: CoverKind,
    source_path: String,
    mime: Option<String>,
    mtime_ns: i64,
    size: i64,
    bytes: Option<Vec<u8>>,
}

/// Blocking walk + tag pass. Call from `spawn_blocking`.
pub fn walk_and_tag(root: &Path, mode: ScanMode, never_split: &[String]) -> io::Result<ScanData> {
    info!(root = root.to_str(), "walk_and_tag started");

    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("library root is not a directory: {}", root.display()),
        ));
    }

    let mut walked = Vec::new();
    collect_files(root, root, &mut walked)?;
    walked.sort_by(|a, b| a.rel.cmp(&b.rel));

    let covers_by_dir = folder_art_by_dir(&walked);

    let files: Vec<ScanFile> = walked
        .iter()
        .map(|file| ScanFile {
            path: file.rel.clone(),
            mtime_ns: file.mtime_ns,
            size: file.size,
            kind: file.kind,
            track_id: None,
        })
        .collect();

    let mut metas: Vec<FileMeta> = Vec::new();
    for file in &walked {
        if file.kind != ScanKind::Audio {
            continue;
        }
        if let Some(meta) = read_file(&file.path, mode, root, file) {
            metas.push(meta);
        }
    }

    let raw: Vec<RawTrack> = metas.iter().map(|meta| meta.raw.clone()).collect();
    let albums = identity::resolve_albums(&raw, never_split);

    Ok(build(albums, metas, files, mode, &covers_by_dir))
}

/// Result of the startup stat walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanCheck {
    Clean,
    NeedsScan(String),
}

/// Walk the root and diff it against the `files` table. This is the cheap
/// startup check; tag reads only happen when the diff is non-empty.
pub fn check(root: &Path, files: &HashMap<String, FileStat>) -> io::Result<ScanCheck> {
    let mut walked = Vec::new();
    collect_files(root, root, &mut walked)?;

    if walked.len() != files.len() {
        return Ok(ScanCheck::NeedsScan(format!(
            "file count changed ({})",
            walked.len()
        )));
    }

    for file in walked {
        let Some(stored) = files.get(&file.rel) else {
            return Ok(ScanCheck::NeedsScan(format!("new file {}", file.rel)));
        };
        if stored.kind != file.kind.as_str()
            || stored.mtime_ns != file.mtime_ns
            || stored.size != file.size
        {
            return Ok(ScanCheck::NeedsScan(format!("changed file {}", file.rel)));
        }
    }

    Ok(ScanCheck::Clean)
}

/// Startup / watcher / manual entry point.
///
/// The manager must have called `begin_scan` for this library before this
/// future runs. The result is returned as a message; the database is the
/// handoff for the actual metadata.
pub async fn library(plan: ScanPlan, force: bool) -> ScanOutcome {
    let library = plan.library.clone();
    // Manager state already rejects overlapping scans; this guard keeps the
    // guarantee even if a caller bypasses it.
    let _guard = plan.scan_lock.clone().lock_owned().await;
    let result = run(plan, force).await;
    ScanOutcome { library, result }
}

async fn run(plan: ScanPlan, force: bool) -> Result<(), String> {
    let LibraryConfig::Offline {
        root,
        scan,
        never_split,
        ..
    } = &plan.config;
    let root = root.clone();
    let mode = *scan;
    let never_split = never_split.clone();
    let signature = ScanSignature::from_config(&plan.config);

    let (pool, cache_dir) = match &plan.handle {
        LibraryHandle::Offline(handle) => (handle.pool.clone(), handle.cache_dir.clone()),
    };
    let db = LibraryDb { pool };

    if !force {
        let complete = db.scan_complete().await.map_err(|err| err.to_string())?;
        if complete {
            let files = db.files().await.map_err(|err| err.to_string())?;
            let check_root = root.clone();
            let outcome = tokio::task::spawn_blocking(move || check(&check_root, &files))
                .await
                .map_err(|err| format!("scan check task failed: {err}"))?
                .map_err(|err| err.to_string())?;
            if matches!(outcome, ScanCheck::Clean) {
                return Ok(());
            }
            tracing::info!(library = %plan.library, reason = ?outcome, "library needs scan");
        }
    }

    let scan_root = root.clone();
    let scan_split = never_split.clone();
    let data = tokio::task::spawn_blocking(move || walk_and_tag(&scan_root, mode, &scan_split))
        .await
        .map_err(|err| format!("scan task failed: {err}"))?
        .map_err(|err| err.to_string())?;

    db.write_scan(&data, &cache_dir, &signature)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<WalkedFile>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        // v1 does not follow symlinks: no loops, no escaping the root.
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_files(root, &path, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let kind = if is_audio(&path) {
            ScanKind::Audio
        } else if is_folder_art(&path) {
            ScanKind::Cover
        } else {
            continue;
        };

        let metadata = entry.metadata()?;
        out.push(WalkedFile {
            rel: rel_id(root, &path),
            path,
            mtime_ns: mtime_ns(&metadata),
            size: metadata.len().min(i64::MAX as u64) as i64,
            kind,
        });
    }
    Ok(())
}

fn folder_art_by_dir(walked: &[WalkedFile]) -> HashMap<String, CoverSource> {
    let mut map = HashMap::new();
    for file in walked {
        if file.kind != ScanKind::Cover {
            continue;
        }
        let Some((dir, _)) = file.rel.rsplit_once('/') else {
            continue;
        };
        if map.contains_key(dir) {
            continue;
        }
        let mime = mime_for_path(&file.path);
        map.insert(
            dir.to_string(),
            CoverSource {
                id: dir.to_string(),
                kind: CoverKind::Folder,
                source_path: file.rel.clone(),
                mime,
                mtime_ns: file.mtime_ns,
                size: file.size,
                bytes: None,
            },
        );
    }
    map
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| AUDIO_EXTENSIONS.iter().any(|a| ext.eq_ignore_ascii_case(a)))
}

fn is_folder_art(path: &Path) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .is_some_and(|name| FOLDER_ART_NAMES.iter().any(|candidate| *candidate == name))
}

fn mime_for_path(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    Some(
        match extension.as_str() {
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "image/jpeg",
        }
        .to_string(),
    )
}

fn rel_id(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn folder_of(rel: &str) -> String {
    rel.rsplit_once('/')
        .map(|(folder, _)| folder.to_string())
        .unwrap_or_default()
}

fn mtime_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

/// One lofty read per file: duration always, names/picture per mode.
fn read_file(path: &Path, mode: ScanMode, root: &Path, walked: &WalkedFile) -> Option<FileMeta> {
    let tagged = match lofty::read_from_path(path) {
        Ok(tagged) => tagged,
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "skipping unreadable file");
            return None;
        }
    };
    let duration = tagged.properties().duration();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let embedded_art = tag.and_then(|tag| tag.pictures().first()).map(|picture| {
        (
            picture.data().to_vec(),
            picture.mime_type().map(|mime| mime.to_string()),
        )
    });

    let raw = match mode {
        ScanMode::Tags => raw_from_tags(path, tag, &walked.rel),
        ScanMode::Folders => raw_from_folders(path, root, tag, &walked.rel),
    };

    Some(FileMeta {
        rel: walked.rel.clone(),
        raw,
        duration,
        mtime_ns: walked.mtime_ns,
        size: walked.size,
        embedded_art,
    })
}

fn raw_from_tags(path: &Path, tag: Option<&Tag>, rel: &str) -> RawTrack {
    let title = tag
        .and_then(|tag| tag.title())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| file_stem(path));
    let compilation = identity::parse_flag(tag_text(tag, ItemKey::FlagCompilation).as_deref());

    RawTrack {
        title,
        album_title: tag_text(tag, ItemKey::AlbumTitle),
        album_artist: tag_text(tag, ItemKey::AlbumArtist),
        album_artist_sort: tag_text(tag, ItemKey::AlbumArtistSortOrder),
        album_artist_mb_id: tag_text(tag, ItemKey::MusicBrainzReleaseArtistId),
        artist_credit: tag
            .and_then(|tag| tag.artist())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        artist_sort: tag_text(tag, ItemKey::TrackArtistSortOrder),
        artist_mb_id: tag_text(tag, ItemKey::MusicBrainzArtistId),
        compilation,
        track_no: tag.and_then(|tag| tag.track()),
        disc_no: tag.and_then(|tag| tag.disk()),
        disc_total: tag.and_then(|tag| tag.disk_total()),
        mb_release_id: tag_text(tag, ItemKey::MusicBrainzReleaseId),
        mb_release_group_id: tag_text(tag, ItemKey::MusicBrainzReleaseGroupId),
        folder: folder_of(rel),
    }
}

fn raw_from_folders(path: &Path, root: &Path, tag: Option<&Tag>, rel: &str) -> RawTrack {
    let (title, track_no, album, artist) =
        match rel_id(root, path).split('/').collect::<Vec<_>>().as_slice() {
            [artist, album, file] => {
                let (title, track_no) = split_track_prefix(&file_stem(Path::new(file)));
                (
                    title,
                    track_no,
                    album.to_string(),
                    artist.trim().to_string(),
                )
            }
            _ => (
                file_stem(path),
                None,
                UNKNOWN_ALBUM.to_string(),
                UNKNOWN_ARTIST.to_string(),
            ),
        };

    RawTrack {
        title,
        album_title: Some(album),
        album_artist: Some(artist.clone()),
        album_artist_sort: None,
        album_artist_mb_id: None,
        artist_credit: Some(artist),
        artist_sort: None,
        artist_mb_id: None,
        compilation: false,
        track_no: track_no.or_else(|| tag.and_then(|tag| tag.track())),
        disc_no: None,
        disc_total: None,
        mb_release_id: None,
        mb_release_group_id: None,
        folder: folder_of(rel),
    }
}

fn tag_text(tag: Option<&Tag>, key: ItemKey) -> Option<String> {
    tag.and_then(|tag| tag.get_string(key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `"01 - Title"` → `("Title", Some(1))`; `"01 Title"` works too.
fn split_track_prefix(stem: &str) -> (String, Option<u32>) {
    let dashed = stem.split_once(" - ");
    let spaced = stem.split_once(' ');
    for (digits, rest) in [dashed, spaced].into_iter().flatten() {
        let rest = rest.trim();
        if !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit())
            && !rest.is_empty()
            && let Ok(n) = digits.parse::<u32>()
        {
            return (rest.to_string(), Some(n));
        }
    }
    (stem.to_string(), None)
}

fn build(
    albums: Vec<ResolvedAlbum>,
    metas: Vec<FileMeta>,
    mut files: Vec<ScanFile>,
    mode: ScanMode,
    covers_by_dir: &HashMap<String, CoverSource>,
) -> ScanData {
    let mut artists: Vec<ScanArtist> = Vec::new();
    let mut artist_index: HashMap<String, usize> = HashMap::new();
    let mut albums_out: Vec<ScanAlbum> = Vec::new();
    let mut tracks: Vec<ScanTrack> = Vec::new();
    let mut covers: Vec<ScanCover> = Vec::new();

    let file_index: HashMap<String, usize> = files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.path.clone(), index))
        .collect();

    for album in albums {
        let album_artist_index = ensure_artist(
            &mut artists,
            &mut artist_index,
            &album.artist_id,
            &album.artist,
            album.artist_sort_name.as_deref(),
            album.artist_mb_id.as_deref(),
        );
        artists[album_artist_index].album_count += 1;

        let cover = pick_cover(&album, &metas, mode, covers_by_dir);
        if let Some(cover) = &cover {
            covers.push(ScanCover {
                id: cover.id.clone(),
                kind: cover.kind,
                source_path: cover.source_path.clone(),
                mime: cover.mime.clone(),
                mtime_ns: cover.mtime_ns,
                size: cover.size,
                bytes: cover.bytes.clone(),
            });
        }
        let cover_id = cover.as_ref().map(|cover| cover.id.clone());

        let duration: Duration = album
            .tracks
            .iter()
            .map(|&index| metas[index].duration)
            .sum();
        albums_out.push(ScanAlbum {
            id: album.id.clone(),
            name: album.name.clone(),
            artist_id: album.artist_id.clone(),
            artist: album.artist.clone(),
            album_sort_name: album.album_sort_name.clone(),
            compilation: album.compilation,
            mb_release_id: album.mb_release_id.clone(),
            mb_release_group_id: album.mb_release_group_id.clone(),
            disc_count: album.disc_count,
            song_count: album.tracks.len() as u32,
            duration: Some(duration),
            cover: cover_id.clone(),
        });

        for &index in &album.tracks {
            let meta = &metas[index];
            let (artist_id, artist) = match &meta.raw.artist_credit {
                Some(credit) => (
                    identity::artist_entity_id(credit, meta.raw.artist_mb_id.as_deref()),
                    credit.clone(),
                ),
                None => (album.artist_id.clone(), album.artist.clone()),
            };
            ensure_artist(
                &mut artists,
                &mut artist_index,
                &artist_id,
                &artist,
                meta.raw.artist_sort.as_deref(),
                meta.raw.artist_mb_id.as_deref(),
            );

            if let Some(&file_index) = file_index.get(meta.rel.as_str()) {
                files[file_index].track_id = Some(meta.rel.clone());
            }
            tracks.push(ScanTrack {
                id: meta.rel.clone(),
                title: meta.raw.title.clone(),
                artist_id,
                artist,
                album_id: album.id.clone(),
                album: album.name.clone(),
                track_no: meta.raw.track_no,
                duration: meta.duration,
                cover: cover_id.clone(),
            });
        }
    }

    ScanData {
        artists,
        albums: albums_out,
        tracks,
        covers,
        files,
    }
}

fn ensure_artist(
    artists: &mut Vec<ScanArtist>,
    index: &mut HashMap<String, usize>,
    id: &str,
    name: &str,
    sort_name: Option<&str>,
    mb_artist_id: Option<&str>,
) -> usize {
    if let Some(&existing) = index.get(id) {
        return existing;
    }
    let position = artists.len();
    artists.push(ScanArtist {
        id: id.to_string(),
        name: name.to_string(),
        sort_name: sort_name
            .map(str::to_string)
            .or_else(|| Some(name.to_string())),
        mb_artist_id: mb_artist_id.map(str::to_string),
        album_count: 0,
        cover: None,
    });
    index.insert(id.to_string(), position);
    position
}

fn pick_cover(
    album: &ResolvedAlbum,
    metas: &[FileMeta],
    mode: ScanMode,
    covers_by_dir: &HashMap<String, CoverSource>,
) -> Option<CoverSource> {
    if mode == ScanMode::Folders {
        let folder = album
            .tracks
            .first()
            .map(|&index| folder_of(&metas[index].rel))
            .unwrap_or_default();
        if let Some(cover) = covers_by_dir.get(&folder) {
            return Some(cover.clone());
        }
    }

    album.tracks.iter().find_map(|&index| {
        let meta = &metas[index];
        meta.embedded_art.as_ref().map(|(bytes, mime)| CoverSource {
            id: meta.rel.clone(),
            kind: CoverKind::Embedded,
            source_path: meta.rel.clone(),
            mime: mime.clone(),
            mtime_ns: meta.mtime_ns,
            size: meta.size,
            bytes: Some(bytes.clone()),
        })
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use lofty::config::WriteOptions;
    use lofty::prelude::Accessor;
    use lofty::tag::{ItemKey, Tag, TagExt, TagType};

    use super::*;
    use crate::library::ScanMode;

    #[test]
    fn splits_dashed_track_prefix() {
        assert_eq!(split_track_prefix("01 - Title"), ("Title".into(), Some(1)));
        assert_eq!(
            split_track_prefix("12 - A Song With Spaces"),
            ("A Song With Spaces".into(), Some(12))
        );
    }

    #[test]
    fn splits_space_track_prefix() {
        assert_eq!(split_track_prefix("02 Title"), ("Title".into(), Some(2)));
        assert_eq!(
            split_track_prefix("003 A Song With Spaces"),
            ("A Song With Spaces".into(), Some(3))
        );
        assert_eq!(
            split_track_prefix("1984 Remaster"),
            ("Remaster".into(), Some(1984))
        );
    }

    #[test]
    fn space_fallback_applies_when_dashed_prefix_is_not_numeric() {
        assert_eq!(
            split_track_prefix("01 Hello - Remix"),
            ("Hello - Remix".into(), Some(1))
        );
    }

    #[test]
    fn passes_through_unprefixed_stems() {
        assert_eq!(
            split_track_prefix("Title Without Number"),
            ("Title Without Number".into(), None)
        );
        assert_eq!(split_track_prefix("01"), ("01".into(), None));
        assert_eq!(split_track_prefix("My - Song"), ("My - Song".into(), None));
        assert_eq!(split_track_prefix("- Title"), ("- Title".into(), None));
    }

    /// Write a tiny valid PCM WAV so lofty can attach an ID3v2 tag.
    fn write_wav(path: &Path) {
        let data_len: u32 = 8;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&8000u32.to_le_bytes()); // sample rate
        bytes.extend_from_slice(&16000u32.to_le_bytes()); // byte rate
        bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        std::fs::write(path, bytes).expect("write wav");
    }

    fn save_tag(path: &Path, build: impl FnOnce(&mut Tag)) {
        let mut tag = Tag::new(TagType::Id3v2);
        build(&mut tag);
        tag.save_to_path(path, WriteOptions::default())
            .expect("save tag");
    }

    #[test]
    fn fixture_scan_groups_compilation_tracks_into_one_album() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("Hits")).expect("mkdir");

        let first = root.join("Hits").join("01 - One.wav");
        write_wav(&first);
        save_tag(&first, |tag| {
            tag.set_title("One".into());
            tag.set_artist("Alice".into());
            tag.set_album("Hits".into());
            tag.set_track(1);
            tag.insert_text(ItemKey::FlagCompilation, "1".into());
        });

        let second = root.join("Hits").join("02 - Two.wav");
        write_wav(&second);
        save_tag(&second, |tag| {
            tag.set_title("Two".into());
            tag.set_artist("Bob".into());
            tag.set_album("Hits".into());
            tag.set_track(2);
            tag.insert_text(ItemKey::FlagCompilation, "1".into());
        });

        let data = walk_and_tag(root, ScanMode::Tags, &[]).expect("scan");
        assert_eq!(data.albums.len(), 1);
        assert_eq!(data.albums[0].artist, identity::VARIOUS_ARTISTS);
        assert!(data.albums[0].compilation);
        assert_eq!(data.tracks.len(), 2);
        assert_eq!(data.tracks[0].artist, "Alice");
        assert_eq!(data.tracks[1].artist, "Bob");
        assert!(data.artists.iter().any(|artist| {
            artist.id == identity::VARIOUS_ARTISTS_ID && artist.album_count == 1
        }));
    }

    #[test]
    fn fixture_scan_preserves_full_track_credit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let path = PathBuf::from(root).join("Song.wav");
        write_wav(&path);
        save_tag(&path, |tag| {
            tag.set_title("Song".into());
            tag.set_artist("Alice feat. Bob".into());
            tag.set_album("Show".into());
        });

        let data = walk_and_tag(root, ScanMode::Tags, &[]).expect("scan");
        assert_eq!(data.tracks.len(), 1);
        assert_eq!(data.tracks[0].artist, "Alice feat. Bob");
        assert_eq!(data.albums[0].artist, "Alice");
    }
}
