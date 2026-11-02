//! Album detail: the selected album's header and tracks.
//!
//! Playback is an intent emitted to `MainScreen`; the screen owns only local
//! display state and cover handles.

use std::time::Duration;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Album, AlbumDetail, AlbumId, CoverId, Manager, Track};
use crate::prelude::*;

use super::fmt_duration;

/// Pixel size requested for the header cover (2x the 128 px display).
const HEADER_COVER_PX: u32 = 256;
const HEADER_COVER_DISPLAY: f32 = 128.0;

#[derive(Debug)]
pub struct State {
    id: AlbumId,
    album: Option<Album>,
    songs: Vec<Track>,
    loaded: bool,
    error: Option<String>,
    cover: CoverState,
}

#[derive(Debug, Clone)]
enum CoverState {
    NotLoaded,
    Loading,
    Loaded(image::Handle),
    Failed,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(AlbumId, Result<AlbumDetail, String>),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    PlayAll,
    PlayTrack(Track),
}

#[derive(Debug, Clone)]
pub enum Event {
    PlayQueue(Vec<Track>),
    PlayTrack(Track),
}

pub fn init(id: AlbumId, manager: Arc<Manager>) -> (State, Task<Msg>) {
    let fetch_id = id.clone();
    let message_id = id.clone();
    let task = Task::perform(
        async move {
            manager
                .album(&fetch_id)
                .await
                .map_err(|err| err.to_string())
        },
        move |result| Msg::Loaded(message_id, result),
    );

    (
        State {
            id,
            album: None,
            songs: Vec::new(),
            loaded: false,
            error: None,
            cover: CoverState::NotLoaded,
        },
        task,
    )
}

pub fn update(state: &mut State, manager: &Arc<Manager>, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Loaded(id, result) => {
            if state.id != id {
                return Action::none();
            }

            state.loaded = true;
            match result {
                Ok(detail) => {
                    let cover = detail.album.cover.clone();
                    state.album = Some(detail.album);
                    state.songs = detail.songs;
                    state.error = None;

                    let Some(cover) = cover else {
                        state.cover = CoverState::NotLoaded;
                        return Action::none();
                    };

                    state.cover = CoverState::Loading;
                    Action::task(cover_task(manager, cover))
                }
                Err(error) => {
                    state.album = None;
                    state.songs.clear();
                    state.cover = CoverState::NotLoaded;
                    state.error = Some(error);
                    Action::none()
                }
            }
        }
        Msg::CoverLoaded(id, result) => {
            if state.album.as_ref().and_then(|album| album.cover.as_ref()) == Some(&id) {
                state.cover = cover_from_result(result);
            }
            Action::none()
        }
        Msg::PlayAll => {
            if state.songs.is_empty() {
                Action::none()
            } else {
                Action::emit(Event::PlayQueue(state.songs.clone()))
            }
        }
        Msg::PlayTrack(track) => Action::emit(Event::PlayTrack(track)),
    }
}

pub fn view(state: &State) -> Element<'_, Msg> {
    if !state.loaded {
        return centered_muted("loading...");
    }

    if let Some(error) = &state.error {
        return centered_muted(error.clone());
    }

    let Some(album) = &state.album else {
        return centered_muted("Album not found");
    };

    let total = state
        .songs
        .iter()
        .fold(Duration::ZERO, |total, track| total + track.duration);
    let artist = album.artist.as_deref().unwrap_or("");
    let mut parts = Vec::new();
    if !artist.is_empty() {
        parts.push(artist.to_string());
    }
    parts.push(format!("{} songs", state.songs.len()));
    parts.push(fmt_duration(total));
    let metadata = parts.join(" · ");

    let header = widget::row![
        cover_view(&state.cover, HEADER_COVER_DISPLAY),
        widget::column![
            text(&album.name).size(24),
            text(metadata).size(12).color([0.6, 0.6, 0.6]),
        ]
        .spacing(4),
        widget::space::horizontal(),
        button("Play album").on_press_maybe((!state.songs.is_empty()).then_some(Msg::PlayAll)),
    ]
    .align_y(Center)
    .spacing(16)
    .padding(Padding {
        bottom: 12.0,
        ..Padding::default()
    });

    let body: Element<'_, Msg> = if state.songs.is_empty() {
        centered_muted("No songs")
    } else {
        let list = table(
            [
                table::column(text("#"), |row: Track| {
                    text(row.track_no.map(|n| n.to_string()).unwrap_or_default())
                        .color([0.6, 0.6, 0.6])
                }),
                table::column(text("Title"), |row: Track| {
                    Element::from(
                        button(text(row.title.clone()))
                            .on_press(Msg::PlayTrack(row.clone()))
                            .width(Fill),
                    )
                })
                .width(Fill),
                table::column(text("Artist"), |row: Track| {
                    text(row.artist.clone().unwrap_or_default()).color([0.65, 0.65, 0.65])
                }),
                table::column(text("Duration"), |row: Track| {
                    text(fmt_duration(row.duration)).color([0.6, 0.6, 0.6])
                }),
                table::column(text(""), |row: Track| {
                    Element::from(
                        button(lucide::play().size(14))
                            .on_press(Msg::PlayTrack(row))
                            .padding(4),
                    )
                }),
            ],
            state.songs.clone(),
        )
        .width(Fill);

        scrollable(list).width(Fill).height(Fill).into()
    };

    widget::column![header, body]
        .width(Fill)
        .height(Fill)
        .into()
}

fn cover_task(manager: &Arc<Manager>, id: CoverId) -> Task<Msg> {
    let manager = Arc::clone(manager);
    let returned_id = id.clone();
    Task::perform(
        async move { manager.cover(&id, HEADER_COVER_PX).await },
        move |result| Msg::CoverLoaded(returned_id, result.map_err(|_| ())),
    )
}

fn cover_from_result(result: Result<Bytes, ()>) -> CoverState {
    match result {
        Ok(bytes) => CoverState::Loaded(image::Handle::from_bytes(bytes)),
        Err(_) => CoverState::Failed,
    }
}

fn cover_view(cover: &CoverState, size: f32) -> Element<'static, Msg> {
    match cover {
        CoverState::Loaded(handle) => image(handle.clone())
            .filter_method(image::FilterMethod::Linear)
            .width(size)
            .height(size)
            .into(),
        _ => widget::container(lucide::music().size(size * 0.4).color([0.5, 0.5, 0.5]))
            .width(size)
            .height(size)
            .align_x(Center)
            .align_y(Center)
            .into(),
    }
}

fn centered_muted(message: impl Into<String>) -> Element<'static, Msg> {
    container(text(message.into()).size(13).color([0.6, 0.6, 0.6]))
        .center(Fill)
        .into()
}
