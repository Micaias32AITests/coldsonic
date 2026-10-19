//! Filesystem walk + tag reads → [`ScanData`].
//!
//! Blocking (lofty is sync) — callers run this inside `smol::unblock`.
//! Produces plain serde records with unprefixed, stable ids: `TrackId` = path
//! relative to the root with `/` separators, `ArtistId` = artist name,
//! `AlbumId` = `artist\u{0}album`, `CoverId` = album directory (folder art) or
//! track path (embedded art). Files that can't be read or tagged are skipped
//! with a warning; a missing root is an error, an empty tree is not.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::prelude::{Accessor, AudioFile, TaggedFileExt};

use crate::library::{ScanMode, primary_artist};

const UNKNOWN_ARTIST: &str = "Unknown Artist";
const UNKNOWN_ALBUM: &str = "Unknown Album";

const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "flac", "ogg", "opus", "m4a", "wav"];

const FOLDER_ART_NAMES: [&str; 4] = ["cover.jpg", "cover.png", "folder.jpg", "folder.png"];

/// Plain serde records; no behavior. Ids here are unprefixed and stable
/// across rescans (paths/names, not row numbers).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ScanData {
    pub(super) artists: Vec<ScanArtist>,
    pub(super) albums: Vec<ScanAlbum>,
    pub(super) tracks: Vec<ScanTrack>,
    pub(super) covers: Vec<ScanCover>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ScanArtist {
    pub(super) name: String,
    pub(super) album_count: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ScanAlbum {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) artist: String,
    pub(super) song_count: u32,
    pub(super) duration: Option<Duration>,
    pub(super) cover: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ScanTrack {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) album: String,
    pub(super) track_no: Option<u32>,
    pub(super) duration: Duration,
    pub(super) cover: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ScanCover {
    pub(super) id: String,
    pub(super) data: Vec<u8>,
}

/// A parsed audio file, before grouping.
struct FileMeta {
    rel: String,
    title: String,
    artist: String,
    album: String,
    track_no: Option<u32>,
    duration: Duration,
    embedded_art: Option<Vec<u8>>,
}

/// One `(artist, album)` grouping; files keep scan (sorted) order inside it.
struct AlbumGroup {
    artist: String,
    name: String,
    tracks: Vec<FileMeta>,
    /// `(cover id, bytes)` — folder art or the first embedded picture found.
    cover: Option<(String, Vec<u8>)>,
}

pub(super) fn scan(root: &Path, mode: ScanMode, never_split: &[String]) -> io::Result<ScanData> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("library root is not a directory: {}", root.display()),
        ));
    }
    let mut files = Vec::new();
    collect_audio_files(root, &mut files)?;
    files.sort();
    let mut groups: Vec<AlbumGroup> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    // First-seen spelling per case-folded artist, so "Kendrick Lamar" and
    // "kendrick lamar" merge into one artist with one canonical name.
    let mut canonical: HashMap<String, String> = HashMap::new();
    for path in files {
        let Some(mut meta) = read_file(&path, mode, root, never_split) else {
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
                    ScanMode::Folders => path
                        .parent()
                        .and_then(|dir| read_folder_art(dir).map(|data| (rel_id(root, dir), data))),
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
            && let Some(art) = meta.embedded_art.take()
        {
            group.cover = Some((meta.rel.clone(), art));
        }
        group.tracks.push(meta);
    }
    Ok(build(groups))
}

fn collect_audio_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_audio_files(&path, out)?;
        } else if is_audio(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| AUDIO_EXTENSIONS.iter().any(|a| ext.eq_ignore_ascii_case(a)))
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

/// One lofty read per file: duration always, names/picture per mode.
///
/// Tag credits go through [`primary_artist`] (feat/`&`/`,` splitting, with
/// the library's `never_split` blacklist); folder names are trim-only — they
/// are deliberate structure, not heuristics.
fn read_file(path: &Path, mode: ScanMode, root: &Path, never_split: &[String]) -> Option<FileMeta> {
    let tagged = match lofty::read_from_path(path) {
        Ok(tagged) => tagged,
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "skipping unreadable file");
            return None;
        }
    };
    let duration = tagged.properties().duration();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let embedded_art = tag
        .and_then(|tag| tag.pictures().first())
        .map(|p| p.data().to_vec());
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
        rel: rel_id(root, path),
        title,
        artist,
        album,
        track_no,
        duration,
        embedded_art,
    })
}

/// `"01 - Title"` → `("Title", Some(1))`; `"01 Title"` works too. The
/// `" - "` form wins when both are present, and the space fallback only
/// applies when it actually starts with a number — anything else passes
/// through unchanged with no track number.
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

/// Case-insensitive `cover.jpg`/`folder.jpg`/… lookup in the album directory.
fn read_folder_art(dir: &Path) -> Option<Vec<u8>> {
    let entry = std::fs::read_dir(dir).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        FOLDER_ART_NAMES
            .iter()
            .any(|candidate| *candidate == name)
            .then(|| entry.path())
    })?;
    match std::fs::read(&entry) {
        Ok(data) => Some(data),
        Err(err) => {
            tracing::warn!(path = %entry.display(), %err, "unreadable folder art");
            None
        }
    }
}

fn build(groups: Vec<AlbumGroup>) -> ScanData {
    let mut artists: Vec<ScanArtist> = Vec::new();
    let mut albums: Vec<ScanAlbum> = Vec::new();
    let mut tracks: Vec<ScanTrack> = Vec::new();
    let mut covers: Vec<ScanCover> = Vec::new();
    for group in groups {
        if !artists.iter().any(|a| a.name == group.artist) {
            artists.push(ScanArtist {
                name: group.artist.clone(),
                album_count: 0,
            });
        }
        let artist = artists
            .iter_mut()
            .find(|a| a.name == group.artist)
            .expect("artist just inserted");
        artist.album_count += 1;

        let duration: Duration = group.tracks.iter().map(|t| t.duration).sum();
        if let Some((cover_id, data)) = &group.cover {
            covers.push(ScanCover {
                id: cover_id.clone(),
                data: data.clone(),
            });
        }
        albums.push(ScanAlbum {
            id: format!("{}\u{0}{}", group.artist, group.name),
            name: group.name.clone(),
            artist: group.artist.clone(),
            song_count: group.tracks.len() as u32,
            duration: Some(duration),
            cover: group.cover.as_ref().map(|(id, _)| id.clone()),
        });
        for track in group.tracks {
            tracks.push(ScanTrack {
                id: track.rel,
                title: track.title,
                artist: track.artist,
                album: track.album,
                track_no: track.track_no,
                duration: track.duration,
                cover: group.cover.as_ref().map(|(id, _)| id.clone()),
            });
        }
    }
    ScanData {
        artists,
        albums,
        tracks,
        covers,
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
        // Same rule as the dashed form: a numeric prefix wins either way.
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
