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

use crate::library::db::library::{CoverKind, FileStat, LibraryDb, ScanSignature};
use crate::library::manager::{LibraryHandle, ScanOutcome, ScanPlan};
use crate::library::{LibraryConfig, ScanMode, primary_artist};

const UNKNOWN_ARTIST: &str = "Unknown Artist";
const UNKNOWN_ALBUM: &str = "Unknown Album";

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
    pub name: String,
    pub album_count: u32,
    pub cover: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScanAlbum {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub song_count: u32,
    pub duration: Option<Duration>,
    pub cover: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScanTrack {
    pub id: String,
    pub title: String,
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

/// A parsed audio file, before grouping.
struct FileMeta {
    rel: String,
    title: String,
    artist: String,
    album: String,
    track_no: Option<u32>,
    duration: Duration,
    mtime_ns: i64,
    size: i64,
    embedded_art: Option<(Vec<u8>, Option<String>)>,
}

/// One `(artist, album)` grouping; files keep scan (sorted) order inside it.
struct AlbumGroup {
    artist: String,
    name: String,
    tracks: Vec<FileMeta>,
    cover: Option<CoverSource>,
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

    let mut files: Vec<ScanFile> = walked
        .iter()
        .map(|file| ScanFile {
            path: file.rel.clone(),
            mtime_ns: file.mtime_ns,
            size: file.size,
            kind: file.kind,
            track_id: None,
        })
        .collect();
    let file_index: HashMap<String, usize> = files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.path.clone(), index))
        .collect();

    let mut groups: Vec<AlbumGroup> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    let mut canonical: HashMap<String, String> = HashMap::new();

    for file in &walked {
        if file.kind != ScanKind::Audio {
            continue;
        }
        let Some(mut meta) = read_file(&file.path, mode, root, never_split, file) else {
            continue;
        };
        let display = canonical
            .entry(meta.artist.to_lowercase())
            .or_insert_with(|| meta.artist.clone())
            .clone();
        meta.artist = display;

        let group_idx = match index.get(&(meta.artist.clone(), meta.album.clone())) {
            Some(&idx) => idx,
            None => {
                let folder_art = match mode {
                    ScanMode::Tags => None,
                    ScanMode::Folders => file
                        .path
                        .parent()
                        .map(|dir| rel_id(root, dir))
                        .and_then(|dir| covers_by_dir.get(&dir).cloned()),
                };
                groups.push(AlbumGroup {
                    artist: meta.artist.clone(),
                    name: meta.album.clone(),
                    tracks: Vec::new(),
                    cover: folder_art,
                });
                index.insert((meta.artist.clone(), meta.album.clone()), groups.len() - 1);
                groups.len() - 1
            }
        };

        let group = &mut groups[group_idx];
        if group.cover.is_none()
            && let Some((bytes, mime)) = meta.embedded_art.take()
        {
            group.cover = Some(CoverSource {
                id: meta.rel.clone(),
                kind: CoverKind::Embedded,
                source_path: meta.rel.clone(),
                mime,
                mtime_ns: meta.mtime_ns,
                size: meta.size,
                bytes: Some(bytes),
            });
        }

        if let Some(&index) = file_index.get(&meta.rel) {
            files[index].track_id = Some(meta.rel.clone());
        }
        group.tracks.push(meta);
    }

    Ok(build(groups, files))
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

fn mtime_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

/// One lofty read per file: duration always, names/picture per mode.
fn read_file(
    path: &Path,
    mode: ScanMode,
    root: &Path,
    never_split: &[String],
    walked: &WalkedFile,
) -> Option<FileMeta> {
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
    let (title, artist, album, track_no) = match mode {
        ScanMode::Tags => match tag {
            Some(tag) => (
                tag.title()
                    .map(|s| s.into_owned())
                    .unwrap_or_else(|| file_stem(path)),
                tag.artist()
                    .map(|s| primary_artist(&s, never_split))
                    .unwrap_or_else(|| UNKNOWN_ARTIST.into()),
                tag.album()
                    .map(|s| s.into_owned())
                    .unwrap_or_else(|| UNKNOWN_ALBUM.into()),
                tag.track(),
            ),
            None => (
                file_stem(path),
                UNKNOWN_ARTIST.into(),
                UNKNOWN_ALBUM.into(),
                None,
            ),
        },
        ScanMode::Folders => match rel_id(root, path).split('/').collect::<Vec<_>>().as_slice() {
            [artist, album, file] => {
                let (title, track_no) = split_track_prefix(&file_stem(Path::new(file)));
                (
                    title,
                    artist.trim().to_string(),
                    album.to_string(),
                    track_no,
                )
            }
            _ => (
                file_stem(path),
                UNKNOWN_ARTIST.into(),
                UNKNOWN_ALBUM.into(),
                None,
            ),
        },
    };
    Some(FileMeta {
        rel: walked.rel.clone(),
        title,
        artist,
        album,
        track_no,
        duration,
        mtime_ns: walked.mtime_ns,
        size: walked.size,
        embedded_art,
    })
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

fn build(groups: Vec<AlbumGroup>, files: Vec<ScanFile>) -> ScanData {
    let mut artists: Vec<ScanArtist> = Vec::new();
    let mut albums: Vec<ScanAlbum> = Vec::new();
    let mut tracks: Vec<ScanTrack> = Vec::new();
    let mut covers: Vec<ScanCover> = Vec::new();

    for group in groups {
        if !artists.iter().any(|a| a.name == group.artist) {
            artists.push(ScanArtist {
                name: group.artist.clone(),
                album_count: 0,
                cover: None,
            });
        }
        let artist = artists
            .iter_mut()
            .find(|a| a.name == group.artist)
            .expect("artist just inserted");
        artist.album_count += 1;

        let duration: Duration = group.tracks.iter().map(|t| t.duration).sum();
        if let Some(cover) = &group.cover {
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
        let album_id = format!("{}\u{0}{}", group.artist, group.name);
        albums.push(ScanAlbum {
            id: album_id.clone(),
            name: group.name.clone(),
            artist: group.artist.clone(),
            song_count: group.tracks.len() as u32,
            duration: Some(duration),
            cover: group.cover.as_ref().map(|cover| cover.id.clone()),
        });
        for track in group.tracks {
            tracks.push(ScanTrack {
                id: track.rel,
                title: track.title,
                artist: track.artist,
                album_id: album_id.clone(),
                album: track.album,
                track_no: track.track_no,
                duration: track.duration,
                cover: group.cover.as_ref().map(|cover| cover.id.clone()),
            });
        }
    }

    ScanData {
        artists,
        albums,
        tracks,
        covers,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::split_track_prefix;

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
}
