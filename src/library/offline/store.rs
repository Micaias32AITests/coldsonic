//! In-memory index built from a scan — a fact, not a cache.
//!
//! Query methods are async (taking `&self`) so genuinely async backends fit
//! without caller changes; today they're cheap clones out of hash maps. Cover
//! `px` is ignored (local files are returned as stored; resize comes later if
//! needed) — the parameter keeps the signature identical to what a networked
//! provider will need.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::scan::ScanData;
use crate::library::{
    Album, AlbumDetail, AlbumId, Artist, ArtistDetail, ArtistId, CoverId, Track, TrackId,
};

#[derive(Debug, Clone)]
pub(super) struct MemoryStore {
    /// Sorted case-insensitively by name; ties keep scan order.
    artists: Vec<Artist>,
    artist_albums: HashMap<ArtistId, Vec<AlbumId>>,
    albums: HashMap<AlbumId, Album>,
    /// Per album, sorted by track number (`None` last) then id.
    album_songs: HashMap<AlbumId, Vec<TrackId>>,
    tracks: HashMap<TrackId, Track>,
    covers: HashMap<CoverId, Vec<u8>>,
}

impl From<ScanData> for MemoryStore {
    fn from(data: ScanData) -> Self {
        let mut artists: Vec<Artist> = data
            .artists
            .into_iter()
            .map(|a| Artist {
                id: ArtistId::new(a.name.clone()),
                name: a.name,
                album_count: a.album_count,
                cover: None,
            })
            .collect();
        artists.sort_by_key(|a| a.name.to_lowercase());

        let albums: HashMap<AlbumId, Album> = data
            .albums
            .into_iter()
            .map(|a| {
                (
                    AlbumId::new(a.id.clone()),
                    Album {
                        id: AlbumId::new(a.id),
                        name: a.name,
                        artist: Some(a.artist),
                        song_count: a.song_count,
                        duration: a.duration,
                        cover: a.cover.map(CoverId::new),
                    },
                )
            })
            .collect();

        let mut tracks: HashMap<TrackId, Track> = HashMap::new();
        let mut album_songs: HashMap<AlbumId, Vec<TrackId>> = HashMap::new();
        for track in data.tracks {
            let album_id = AlbumId::new(format!("{}\u{0}{}", track.artist, track.album));
            let id = TrackId::new(track.id.clone());
            let record = Track {
                id: id.clone(),
                title: track.title,
                artist: Some(track.artist),
                album: Some(track.album),
                track_no: track.track_no,
                duration: track.duration,
                cover: track.cover.map(CoverId::new),
            };
            tracks.insert(id.clone(), record);
            album_songs.entry(album_id).or_default().push(id);
        }
        for songs in album_songs.values_mut() {
            songs.sort_by(|a, b| compare_song_order(a, b, &tracks));
        }

        let mut artist_albums: HashMap<ArtistId, Vec<AlbumId>> = HashMap::new();
        for album in albums.values() {
            let artist_id = ArtistId::new(album.artist.clone().unwrap_or_default());
            artist_albums
                .entry(artist_id)
                .or_default()
                .push(album.id.clone());
        }
        for list in artist_albums.values_mut() {
            list.sort_by_key(|id| {
                albums
                    .get(id)
                    .map(|a| a.name.to_lowercase())
                    .unwrap_or_default()
            });
        }

        let covers: HashMap<CoverId, Vec<u8>> = data
            .covers
            .into_iter()
            .map(|c| (CoverId::new(c.id), c.data))
            .collect();

        Self {
            artists,
            artist_albums,
            albums,
            album_songs,
            tracks,
            covers,
        }
    }
}

fn compare_song_order(a: &TrackId, b: &TrackId, tracks: &HashMap<TrackId, Track>) -> Ordering {
    let a_no = tracks.get(a).and_then(|t| t.track_no);
    let b_no = tracks.get(b).and_then(|t| t.track_no);
    match (a_no, b_no) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.as_str().cmp(b.as_str())),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.as_str().cmp(b.as_str()),
    }
}

impl MemoryStore {
    pub(super) async fn artists(&self) -> Vec<Artist> {
        self.artists.clone()
    }

    pub(super) async fn artist(&self, id: &ArtistId) -> Option<ArtistDetail> {
        let artist = self.artists.iter().find(|a| a.id == *id)?.clone();
        let albums = self
            .artist_albums
            .get(id)?
            .iter()
            .filter_map(|id| self.albums.get(id).cloned())
            .collect();
        Some(ArtistDetail { artist, albums })
    }

    pub(super) async fn album(&self, id: &AlbumId) -> Option<AlbumDetail> {
        let album = self.albums.get(id)?.clone();
        let songs = self.album_songs(id).await?;
        Some(AlbumDetail { album, songs })
    }

    pub(super) async fn album_songs(&self, id: &AlbumId) -> Option<Vec<Track>> {
        let ids = self.album_songs.get(id)?;
        Some(
            ids.iter()
                .filter_map(|id| self.tracks.get(id).cloned())
                .collect(),
        )
    }

    pub(super) async fn cover_bytes(&self, id: &CoverId, _px: u32) -> Option<Vec<u8>> {
        self.covers.get(id).cloned()
    }
}
