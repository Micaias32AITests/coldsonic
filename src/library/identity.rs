//! Album identity resolution.
//!
//! Pure logic: no filesystem, no lofty, no database. The scanner reads tags
//! into [`RawTrack`] values and this module turns them into albums using the
//! priority documented in `change_docs/metadata_01.md`:
//!
//! 1. MusicBrainz release id
//! 2. MusicBrainz release-group id
//! 3. album artist + album title
//! 4. consistent primary track artist + album title
//! 5. folder + album title
//! 6. folder alone
//! 7. Various Artists / Unknown fallback
//!
//! Strong identities (MusicBrainz ids) are grouped first. Weaker identities
//! only decide the remaining tracks, except that an untagged track sharing a
//! folder and album title with a strong group may join it, provided its album
//! artist does not conflict.

use std::collections::HashMap;

use crate::library::primary_artist;

pub(crate) const VARIOUS_ARTISTS: &str = "Various Artists";
pub(crate) const UNKNOWN_ARTIST: &str = "Unknown Artist";
pub(crate) const UNKNOWN_ALBUM: &str = "Unknown Album";
pub(crate) const VARIOUS_ARTISTS_ID: &str = "various-artists";

/// One track's raw metadata, before grouping.
#[derive(Debug, Clone, Default)]
pub(crate) struct RawTrack {
    pub title: String,
    pub album_title: Option<String>,
    /// Literal `ALBUMARTIST` credit, kept whole.
    pub album_artist: Option<String>,
    pub album_artist_sort: Option<String>,
    pub album_artist_mb_id: Option<String>,
    /// Literal `ARTIST` credit, kept whole for display.
    pub artist_credit: Option<String>,
    pub artist_sort: Option<String>,
    pub artist_mb_id: Option<String>,
    pub compilation: bool,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub disc_total: Option<u32>,
    pub mb_release_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    /// Relative folder, `/` separated, without the file name.
    pub folder: String,
}

/// The album a candidate set resolved to.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedAlbum {
    pub id: String,
    pub name: String,
    pub artist_id: String,
    pub artist: String,
    pub artist_sort_name: Option<String>,
    pub artist_mb_id: Option<String>,
    pub album_sort_name: Option<String>,
    pub compilation: bool,
    pub mb_release_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub disc_count: Option<u32>,
    /// Indexes into the `tracks` slice passed to [`resolve_albums`].
    pub tracks: Vec<usize>,
}

/// Resolve every track into exactly one album.
pub(crate) fn resolve_albums(tracks: &[RawTrack], never_split: &[String]) -> Vec<ResolvedAlbum> {
    let mut group_of: Vec<Option<usize>> = vec![None; tracks.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();

    // 1. MusicBrainz release id.
    for (index, track) in tracks.iter().enumerate() {
        if let Some(id) = normalized_uuid(track.mb_release_id.as_deref()) {
            assign(
                &mut group_of,
                &mut groups,
                &mut by_key,
                index,
                format!("release\0{id}"),
            );
        }
    }

    // 2. MusicBrainz release-group id.
    for (index, track) in tracks.iter().enumerate() {
        if group_of[index].is_some() {
            continue;
        }
        if let Some(id) = normalized_uuid(track.mb_release_group_id.as_deref()) {
            assign(
                &mut group_of,
                &mut groups,
                &mut by_key,
                index,
                format!("release-group\0{id}"),
            );
        }
    }

    // 3. A track without any MusicBrainz id may still belong to a strong group
    //    when it shares the folder and album title and does not contradict the
    //    group's album artist.
    for (index, track) in tracks.iter().enumerate() {
        if group_of[index].is_some() {
            continue;
        }
        let Some(title) = title_key(track) else {
            continue;
        };
        let folder = folder_key(&track.folder);
        let mut found = None;
        for (group_index, members) in groups.iter().enumerate() {
            let joins = members.iter().any(|&member| {
                title_key(&tracks[member]).as_deref() == Some(title.as_str())
                    && folder_key(&tracks[member].folder) == folder
                    && album_artist_compatible(&tracks[member], track)
            });
            if joins {
                found = Some(group_index);
                break;
            }
        }
        if let Some(group_index) = found {
            group_of[index] = Some(group_index);
            groups[group_index].push(index);
        }
    }

    // 4. Album artist + album title.
    for (index, track) in tracks.iter().enumerate() {
        if group_of[index].is_some() {
            continue;
        }
        let Some(artist) = normalized(track.album_artist.as_deref().unwrap_or("")) else {
            continue;
        };
        assign(
            &mut group_of,
            &mut groups,
            &mut by_key,
            index,
            format!(
                "album-artist\0{artist}\0{}",
                title_key(track).unwrap_or_default()
            ),
        );
    }

    // 5. Folder + album title (and folder alone when there is no title).
    for (index, track) in tracks.iter().enumerate() {
        if group_of[index].is_some() {
            continue;
        }
        let folder = folder_key(&track.folder);
        let key = match title_key(track) {
            Some(title) => format!("folder\0{folder}\0{title}"),
            None => format!("folder\0{folder}"),
        };
        assign(&mut group_of, &mut groups, &mut by_key, index, key);
    }

    groups
        .iter()
        .map(|members| resolve_group(tracks, members, never_split))
        .collect()
}

fn assign(
    group_of: &mut [Option<usize>],
    groups: &mut Vec<Vec<usize>>,
    by_key: &mut HashMap<String, usize>,
    index: usize,
    key: String,
) {
    let group_index = *by_key.entry(key).or_insert_with(|| {
        groups.push(Vec::new());
        groups.len() - 1
    });
    groups[group_index].push(index);
    group_of[index] = Some(group_index);
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    MbRelease(String),
    MbReleaseGroup(String),
    AlbumArtist { normalized: String },
    Artist { normalized: String },
    Various,
    FolderTitle { folder: String, title: String },
    Folder { folder: String },
    Unknown,
}

fn resolve_group(tracks: &[RawTrack], members: &[usize], never_split: &[String]) -> ResolvedAlbum {
    let release_id =
        single_normalized_uuid(members, tracks, |track| track.mb_release_id.as_deref());
    let release_group_id = single_normalized_uuid(members, tracks, |track| {
        track.mb_release_group_id.as_deref()
    });
    let album_artist_names = single_values(members, tracks, |track| track.album_artist.as_deref());
    // Display spelling: keep the first literal credit, not the normalized key.
    let album_artist = members
        .iter()
        .find_map(|&member| clean(tracks[member].album_artist.as_deref()));

    let primary_artists = {
        let mut values: Vec<String> = Vec::new();
        for &member in members {
            if let Some(credit) = tracks[member].artist_credit.as_deref() {
                let primary = primary_artist(credit, never_split);
                if let Some(normalized) = normalized(&primary)
                    && !values.contains(&normalized)
                {
                    values.push(normalized);
                }
            }
        }
        values
    };
    let any_compilation = members.iter().any(|&member| tracks[member].compilation);

    let folder = folder_key(&tracks[members[0]].folder);
    let title = title_key(&tracks[members[0]]);

    let kind = if let Some(id) = release_id.clone() {
        Kind::MbRelease(id)
    } else if let Some(id) = release_group_id.clone() {
        Kind::MbReleaseGroup(id)
    } else if album_artist_names.len() == 1 {
        Kind::AlbumArtist {
            normalized: album_artist_names[0].clone(),
        }
    } else if primary_artists.len() == 1 {
        Kind::Artist {
            normalized: primary_artists[0].clone(),
        }
    } else if primary_artists.len() > 1 || any_compilation {
        Kind::Various
    } else if let Some(title) = title.clone() {
        Kind::FolderTitle { folder, title }
    } else if !folder.is_empty() {
        Kind::Folder { folder }
    } else {
        Kind::Unknown
    };

    let explicit_album_artist = matches!(kind, Kind::AlbumArtist { .. });
    let display_artist = match &kind {
        Kind::AlbumArtist { .. } => album_artist
            .clone()
            .unwrap_or_else(|| UNKNOWN_ARTIST.into()),
        Kind::Artist { .. } => {
            // Recover the display spelling from the first matching credit.
            members
                .iter()
                .find_map(|&member| {
                    let credit = tracks[member].artist_credit.as_deref()?;
                    let primary = primary_artist(credit, never_split);
                    normalized(&primary)
                        .filter(|value| Some(value) == primary_artists.first())
                        .map(|_| primary)
                })
                .unwrap_or_else(|| UNKNOWN_ARTIST.into())
        }
        Kind::Various => VARIOUS_ARTISTS.into(),
        _ => UNKNOWN_ARTIST.into(),
    };

    let id = match &kind {
        Kind::MbRelease(id) => format!("mb-release:{id}"),
        Kind::MbReleaseGroup(id) => format!("mb-release-group:{id}"),
        Kind::AlbumArtist { normalized } => {
            format!(
                "album-artist:{normalized}\0album:{}",
                title.clone().unwrap_or_default()
            )
        }
        Kind::Artist { normalized } => {
            format!(
                "artist:{normalized}\0album:{}",
                title.clone().unwrap_or_default()
            )
        }
        Kind::Various => format!(
            "{VARIOUS_ARTISTS_ID}\0album:{}",
            title.clone().unwrap_or_default()
        ),
        Kind::FolderTitle { folder, title } => format!("folder:{folder}\0album:{title}"),
        Kind::Folder { folder } => format!("folder:{folder}"),
        Kind::Unknown => "unknown-album".into(),
    };

    let artist_mb_id = match &kind {
        Kind::Various | Kind::Unknown => None,
        Kind::AlbumArtist { .. } => members
            .iter()
            .find_map(|&member| normalized_uuid(tracks[member].album_artist_mb_id.as_deref())),
        _ => members
            .iter()
            .find_map(|&member| normalized_uuid(tracks[member].artist_mb_id.as_deref())),
    };
    let (artist_id, artist) = match &kind {
        Kind::Various => (VARIOUS_ARTISTS_ID.to_string(), VARIOUS_ARTISTS.to_string()),
        Kind::Unknown => (
            artist_entity_id(UNKNOWN_ARTIST, None),
            UNKNOWN_ARTIST.into(),
        ),
        _ => (
            artist_entity_id(&display_artist, artist_mb_id.as_deref()),
            display_artist.clone(),
        ),
    };

    let artist_sort_name = if explicit_album_artist {
        members
            .iter()
            .find_map(|&member| tracks[member].album_artist_sort.clone())
            .or_else(|| Some(display_artist.clone()))
    } else {
        Some(display_artist.clone())
    };
    let album_sort_name = artist_sort_name.clone();

    let compilation = matches!(kind, Kind::Various) || (any_compilation && !explicit_album_artist);

    let name = display_title(members, tracks);

    let disc_count = members
        .iter()
        .flat_map(|&member| {
            let track = &tracks[member];
            [track.disc_no, track.disc_total]
        })
        .flatten()
        .max()
        .map(|value| value.max(1));

    ResolvedAlbum {
        id,
        name,
        artist_id,
        artist,
        artist_sort_name,
        artist_mb_id,
        album_sort_name,
        compilation,
        mb_release_id: release_id,
        mb_release_group_id: release_group_id,
        disc_count,
        tracks: members.to_vec(),
    }
}

fn display_title(members: &[usize], tracks: &[RawTrack]) -> String {
    members
        .iter()
        .find_map(|&member| display_album_title(&tracks[member]))
        .unwrap_or_else(|| UNKNOWN_ALBUM.into())
}

fn display_album_title(track: &RawTrack) -> Option<String> {
    clean(track.album_title.as_deref()).or_else(|| folder_title(&track.folder))
}

fn title_key(track: &RawTrack) -> Option<String> {
    normalized(&display_album_title(track).unwrap_or_default())
}

fn folder_title(folder: &str) -> Option<String> {
    folder
        .split('/')
        .rev()
        .find(|part| !part.trim().is_empty())
        .map(|part| part.trim().to_string())
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalized(value: &str) -> Option<String> {
    let words: Vec<String> = value
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

fn normalized_uuid(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_lowercase())
    }
}

fn folder_key(folder: &str) -> String {
    folder.trim_matches('/').to_string()
}

fn album_artist_compatible(left: &RawTrack, right: &RawTrack) -> bool {
    match (
        normalized(left.album_artist.as_deref().unwrap_or("")),
        normalized(right.album_artist.as_deref().unwrap_or("")),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

/// All distinct normalized values that are present, preserving display order.
fn single_values(
    members: &[usize],
    tracks: &[RawTrack],
    value: impl Fn(&RawTrack) -> Option<&str>,
) -> Vec<String> {
    let mut values: Vec<String> = Vec::new();
    for &member in members {
        if let Some(normalized) = normalized(value(&tracks[member]).unwrap_or(""))
            && !values.iter().any(|existing| existing == &normalized)
        {
            values.push(normalized);
        }
    }
    values
}

fn single_normalized_uuid(
    members: &[usize],
    tracks: &[RawTrack],
    value: impl Fn(&RawTrack) -> Option<&str>,
) -> Option<String> {
    let mut found: Option<String> = None;
    for &member in members {
        let Some(id) = normalized_uuid(value(&tracks[member])) else {
            continue;
        };
        match &found {
            Some(existing) if existing != &id => return None,
            Some(_) => {}
            None => found = Some(id),
        }
    }
    found
}

/// The stable id for one artist entity: MusicBrainz id when present, otherwise
/// the normalized display name.
pub(crate) fn artist_entity_id(name: &str, mb_id: Option<&str>) -> String {
    if let Some(id) = normalized_uuid(mb_id) {
        format!("mb:{id}")
    } else {
        format!(
            "name:{}",
            normalized(name).unwrap_or_else(|| "unknown-artist".into())
        )
    }
}

/// Accept the common truthy spellings of the compilation flag.
pub(crate) fn parse_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artist: &str, album: &str) -> RawTrack {
        RawTrack {
            artist_credit: Some(artist.into()),
            album_title: Some(album.into()),
            album_artist: None,
            folder: "folder".into(),
            ..RawTrack::default()
        }
    }

    fn ids(albums: &[ResolvedAlbum]) -> Vec<&str> {
        albums.iter().map(|album| album.id.as_str()).collect()
    }

    #[test]
    fn same_album_artist_groups_different_track_artists() {
        let mut a = track("Alice", "Shared");
        a.album_artist = Some("Alice".into());
        let mut b = track("Bob", "Shared");
        b.album_artist = Some("Alice".into());
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist, "Alice");
    }

    #[test]
    fn collaboration_album_keeps_credit_whole() {
        let mut a = track("Alice", "Together");
        a.album_artist = Some("Alice & Bob".into());
        let mut b = track("Bob", "Together");
        b.album_artist = Some("Alice & Bob".into());
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist, "Alice & Bob");
    }

    #[test]
    fn compilation_flag_without_album_artist_is_various_artists() {
        let mut a = track("Alice", "Hits");
        a.compilation = true;
        let mut b = track("Bob", "Hits");
        b.compilation = true;
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist, VARIOUS_ARTISTS);
        assert!(albums[0].compilation);
        assert_eq!(albums[0].artist_id, VARIOUS_ARTISTS_ID);
    }

    #[test]
    fn multiple_primary_artists_without_album_artist_is_various_artists() {
        let a = track("Alice", "Shared");
        let b = track("Bob", "Shared");
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist, VARIOUS_ARTISTS);
    }

    #[test]
    fn featured_artists_with_same_primary_stay_one_artist_album() {
        let a = track("Alice feat. Bob", "Show");
        let b = track("Alice feat. Carol", "Show");
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist, "Alice");
    }

    #[test]
    fn same_title_different_artists_different_folders_are_separate() {
        let mut a = track("Alice", "Greatest Hits");
        a.folder = "Alice".into();
        let mut b = track("Bob", "Greatest Hits");
        b.folder = "Bob".into();
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn release_id_wins_over_folders() {
        let mut a = track("Alice", "Album");
        a.folder = "Disc 1".into();
        a.mb_release_id = Some("ABC".into());
        let mut b = track("Alice", "Album");
        b.folder = "Disc 2".into();
        b.mb_release_id = Some("abc".into());
        let mut c = track("Alice", "Album");
        c.folder = "Other".into();
        c.mb_release_id = Some("other".into());
        let albums = resolve_albums(&[a, b, c], &[]);
        let release: Vec<_> = albums
            .iter()
            .filter(|album| album.id == "mb-release:abc")
            .collect();
        assert_eq!(release.len(), 1);
        assert_eq!(release[0].tracks.len(), 2);
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn release_group_wins_when_release_ids_are_absent() {
        let mut a = track("Alice", "Album");
        a.mb_release_group_id = Some("GROUP".into());
        let mut b = track("Alice", "Album");
        b.mb_release_group_id = Some("group".into());
        b.folder = "Elsewhere".into();
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(ids(&albums), vec!["mb-release-group:group"]);
    }

    #[test]
    fn missing_tags_fall_back_to_folder() {
        let a = RawTrack {
            folder: "Hits".into(),
            ..RawTrack::default()
        };
        let b = RawTrack {
            folder: "Hits".into(),
            ..RawTrack::default()
        };
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name, "Hits");
    }

    #[test]
    fn untagged_track_joins_release_sharing_folder_and_title() {
        let mut tagged = track("Alice", "Album");
        tagged.mb_release_id = Some("uuid".into());
        tagged.folder = "Album".into();
        tagged.album_artist = Some("Alice".into());
        let mut untagged = track("Alice", "Album");
        untagged.folder = "Album".into();
        untagged.album_artist = Some("Alice".into());
        let albums = resolve_albums(&[tagged, untagged], &[]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].id, "mb-release:uuid");
    }

    #[test]
    fn conflicting_album_artist_does_not_join_release() {
        let mut tagged = track("Alice", "Album");
        tagged.mb_release_id = Some("uuid".into());
        tagged.folder = "Album".into();
        tagged.album_artist = Some("Alice".into());
        let mut other = track("Bob", "Album");
        other.folder = "Album".into();
        other.album_artist = Some("Bob".into());
        let albums = resolve_albums(&[tagged, other], &[]);
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn compilation_flag_is_overridden_by_explicit_album_artist() {
        let mut a = track("Alice", "Album");
        a.compilation = true;
        a.album_artist = Some("Alice".into());
        let albums = resolve_albums(&[a], &[]);
        assert_eq!(albums[0].artist, "Alice");
        assert!(!albums[0].compilation);
    }

    #[test]
    fn never_split_protects_album_artist_names() {
        let keep = ["Simon & Garfunkel".to_string()];
        let mut a = track("Simon & Garfunkel", "Album");
        a.album_artist = Some("Simon & Garfunkel".into());
        let albums = resolve_albums(&[a], &keep);
        assert_eq!(albums[0].artist, "Simon & Garfunkel");
    }

    #[test]
    fn parse_flag_accepts_common_spellings() {
        assert!(parse_flag(Some("1")));
        assert!(parse_flag(Some("TRUE")));
        assert!(parse_flag(Some("yes")));
        assert!(parse_flag(Some("on")));
        assert!(!parse_flag(Some("0")));
        assert!(!parse_flag(None));
    }

    #[test]
    fn artist_id_uses_musicbrainz_when_present() {
        assert_eq!(artist_entity_id("Alice", Some("ABC")), "mb:abc");
        assert_eq!(artist_entity_id("Alice", None), "name:alice");
    }

    #[test]
    fn disc_count_is_the_maximum_seen() {
        let mut a = track("Alice", "Album");
        a.album_artist = Some("Alice".into());
        a.disc_no = Some(1);
        a.disc_total = Some(2);
        let mut b = track("Alice", "Album");
        b.album_artist = Some("Alice".into());
        b.disc_no = Some(2);
        let albums = resolve_albums(&[a, b], &[]);
        assert_eq!(albums[0].disc_count, Some(2));
    }
}
