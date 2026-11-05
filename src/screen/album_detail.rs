//! Album detail: the selected album's header and tracks.
//!
//! Tracks use the same virtualized list as `all_tracks`, so opening a large
//! album does not lay out every row before the first frame.

use std::time::Duration;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{AlbumDetail, AlbumId, CoverId, Manager, ManagerHandle, Track};
use crate::prelude::*;
use crate::widget::lazy_list::lazy_list;
use crate::widget::text_button::text_button;

use super::fmt_duration;
use super::load_state::{LoadError, LoadState};

/// Pixel size requested for the header cover (2x the 128 px display).
const HEADER_COVER_PX: u32 = 256;
const HEADER_COVER_DISPLAY: f32 = 128.0;

const ROW_HEIGHT: f32 = 56.0;

#[derive(Debug)]
pub struct State {
    id: AlbumId,
    load: LoadState<AlbumDetail>,
    cover: CoverState,
    revision: u64,
    dirty: bool,
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
    Loaded {
        revision: u64,
        id: AlbumId,
        result: Result<AlbumDetail, LoadError>,
    },
    Reload,
    CoverLoaded(CoverId, Result<Bytes, ()>),
    PlayAll,
    PlayTrack(Track),
}

#[derive(Debug, Clone)]
pub enum Event {
    PlayQueue(Vec<Track>),
    PlayTrack(Track),
}

pub fn init(id: AlbumId, handle: ManagerHandle, revision: u64) -> (State, Task<Msg>) {
    let task = fetch(id.clone(), handle, revision);
    (
        State {
            id,
            load: LoadState::Loading,
            cover: CoverState::NotLoaded,
            revision,
            dirty: false,
        },
        task,
    )
}

fn fetch(id: AlbumId, handle: ManagerHandle, revision: u64) -> Task<Msg> {
    let message_id = id.clone();
    Task::perform(async move { handle.album(&id).await }, move |result| {
        Msg::Loaded {
            revision,
            id: message_id,
            result: result.map_err(LoadError::from),
        }
    })
}

pub fn update(state: &mut State, manager: &Manager, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Reload => {
            state.dirty = true;
            Action::task(fetch(state.id.clone(), manager.handle(), state.revision))
        }
        Msg::Loaded {
            revision,
            id,
            result,
        } => {
            if revision != state.revision || state.id != id {
                return Action::none();
            }
            state.dirty = false;
            state.load = super::load_state::load_state(result);
            match &state.load {
                LoadState::Loaded(detail) => {
                    let cover = detail.album.cover.clone();
                    let Some(cover) = cover else {
                        state.cover = CoverState::NotLoaded;
                        return Action::none();
                    };
                    state.cover = CoverState::Loading;
                    Action::task(cover_task(manager.handle(), cover))
                }
                _ => {
                    state.cover = CoverState::NotLoaded;
                    Action::none()
                }
            }
        }
        Msg::CoverLoaded(id, result) => {
            if state
                .load
                .loaded()
                .and_then(|detail| detail.album.cover.as_ref())
                == Some(&id)
            {
                state.cover = cover_from_result(result);
            }
            Action::none()
        }
        Msg::PlayAll => match state.load.loaded() {
            Some(detail) if !detail.songs.is_empty() => {
                Action::emit(Event::PlayQueue(detail.songs.clone()))
            }
            _ => Action::none(),
        },
        Msg::PlayTrack(track) => Action::emit(Event::PlayTrack(track)),
    }
}

impl State {
    pub fn mark_dirty(&mut self, revision: u64) {
        self.revision = revision;
        self.dirty = true;
        self.cover = CoverState::NotLoaded;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

pub fn view(state: &State) -> Element<'_, Msg> {
    let detail = match &state.load {
        LoadState::Loading => return centered_muted("loading..."),
        LoadState::NotFound => return centered_muted("Album not found"),
        LoadState::Error(error) => return centered_muted(error.clone()),
        LoadState::Loaded(detail) => detail,
    };

    let total = detail
        .songs
        .iter()
        .fold(Duration::ZERO, |total, track| total + track.duration);
    let artist = detail.album.artist.as_deref().unwrap_or("");
    let mut parts = Vec::new();
    if !artist.is_empty() {
        parts.push(artist.to_string());
    }
    parts.push(format!("{} songs", detail.songs.len()));
    parts.push(fmt_duration(total));
    let metadata = parts.join(" · ");

    let header = widget::row![
        cover_view(&state.cover, HEADER_COVER_DISPLAY),
        widget::column![
            text(&detail.album.name).size(24),
            text(metadata).size(12).color([0.6, 0.6, 0.6]),
        ]
        .spacing(4),
        widget::space::horizontal(),
        button("Play album").on_press_maybe((!detail.songs.is_empty()).then_some(Msg::PlayAll)),
    ]
    .align_y(Center)
    .spacing(16)
    .padding(Padding {
        bottom: 12.0,
        ..Padding::default()
    });

    let body: Element<'_, Msg> = if detail.songs.is_empty() {
        centered_muted("No songs")
    } else {
        let list = lazy_list(&detail.songs, ROW_HEIGHT, |_index, row: &Track| {
            track_row(row)
        })
        .width(Fill)
        .height(Fill);

        widget::column![track_header(), list]
            .width(Fill)
            .height(Fill)
            .into()
    };

    widget::column![header, body]
        .width(Fill)
        .height(Fill)
        .into()
}

fn track_header() -> Element<'static, Msg> {
    widget::row![
        text("#").size(11).color([0.55, 0.55, 0.55]).width(32),
        text("Title")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(4)),
        text("Artist")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(3)),
        text("Duration")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(80),
        widget::container(text("")).width(32),
    ]
    .spacing(10)
    .padding(Padding::from([4, 8]))
    .into()
}

fn track_row<'a>(row: &'a Track) -> Element<'a, Msg> {
    widget::row![
        text(row.track_no.map(|n| n.to_string()).unwrap_or_default())
            .color([0.6, 0.6, 0.6])
            .width(32),
        text_button(
            text(&row.title)
                .size(14)
                .wrapping(iced::widget::text::Wrapping::None)
        )
        .on_press(Msg::PlayTrack(row.clone()))
        .width(FillPortion(4)),
        text(row.artist.as_deref().unwrap_or(""))
            .size(12)
            .color([0.65, 0.65, 0.65])
            .wrapping(iced::widget::text::Wrapping::None)
            .width(FillPortion(3)),
        text(fmt_duration(row.duration))
            .size(12)
            .color([0.6, 0.6, 0.6])
            .width(80),
        button(lucide::play().size(14))
            .on_press(Msg::PlayTrack(row.clone()))
            .padding(4),
    ]
    .align_y(Center)
    .spacing(10)
    .padding(Padding::from([4, 8]))
    .height(ROW_HEIGHT)
    .width(Fill)
    .into()
}

fn cover_task(handle: ManagerHandle, id: CoverId) -> Task<Msg> {
    let returned_id = id.clone();
    Task::perform(
        async move { handle.cover(&id, HEADER_COVER_PX).await },
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
