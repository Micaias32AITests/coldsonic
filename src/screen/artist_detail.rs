//! Artist detail: the selected artist's header and albums.
//!
//! The album list is virtualized so a prolific artist's page does not shape
//! every row on the UI thread before the first frame appears.

use std::ops::Range;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Album, AlbumId, ArtistDetail, ArtistId, CoverId, Manager, ManagerHandle};
use crate::prelude::*;
use crate::widget::lazy_list::{lazy_list, visible_range};

use super::covers::{CoverMap, CoverState};
use super::load_state::{LoadError, LoadState};

/// Pixel size requested for the header cover (2x the 128 px display).
const HEADER_COVER_PX: u32 = 256;
const HEADER_COVER_DISPLAY: f32 = 128.0;

/// Pixel size requested for row covers (2x the 48 px display).
const ROW_COVER_DISPLAY: f32 = 48.0;

const ROW_HEIGHT: f32 = 56.0;
const INITIAL_ROWS: usize = 32;
const OVERSCAN: usize = 3;

#[derive(Debug)]
pub struct State {
    id: ArtistId,
    load: LoadState<ArtistDetail>,
    covers: CoverMap,
    visible: Range<usize>,
    revision: u64,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded {
        revision: u64,
        id: ArtistId,
        result: Result<ArtistDetail, LoadError>,
    },
    Reload,
    Scrolled(iced::widget::scrollable::Viewport),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    AlbumClicked(AlbumId),
    Shuffle,
}

#[derive(Debug, Clone)]
pub enum Event {
    AlbumClicked(AlbumId),
    PlayShuffled(ArtistId),
}

pub fn init(id: ArtistId, handle: ManagerHandle, revision: u64) -> (State, Task<Msg>) {
    let task = fetch(id.clone(), handle, revision);
    (
        State {
            id,
            load: LoadState::Loading,
            covers: CoverMap::default(),
            visible: 0..INITIAL_ROWS,
            revision,
            dirty: false,
        },
        task,
    )
}

fn fetch(id: ArtistId, handle: ManagerHandle, revision: u64) -> Task<Msg> {
    let message_id = id.clone();
    Task::perform(async move { handle.artist(&id).await }, move |result| {
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
            state.covers.clear();
            state.load = super::load_state::load_state(result);
            state.visible = 0..INITIAL_ROWS;
            refresh_covers(state, manager)
        }
        Msg::Scrolled(viewport) => {
            state.visible = visible_range(&viewport, ROW_HEIGHT, OVERSCAN);
            refresh_covers(state, manager)
        }
        Msg::CoverLoaded(id, result) => {
            state.covers.apply(id, result);
            Action::none()
        }
        Msg::AlbumClicked(id) => Action::emit(Event::AlbumClicked(id)),
        Msg::Shuffle => Action::emit(Event::PlayShuffled(state.id.clone())),
    }
}

impl State {
    pub fn mark_dirty(&mut self, revision: u64) {
        self.revision = revision;
        self.dirty = true;
        self.covers.clear();
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

pub fn view(state: &State) -> Element<'_, Msg> {
    let detail = match &state.load {
        LoadState::Loading => return centered_muted("loading..."),
        LoadState::NotFound => return centered_muted("Artist not found"),
        LoadState::Error(error) => return centered_muted(error.clone()),
        LoadState::Loaded(detail) => detail,
    };

    let header_cover = detail
        .artist
        .cover
        .as_ref()
        .and_then(|id| state.covers.get(id));
    let header = widget::row![
        header_cover_view(header_cover, HEADER_COVER_DISPLAY),
        widget::column![
            text(&detail.artist.name).size(24),
            text(format!("{} albums", detail.artist.album_count))
                .size(12)
                .color([0.6, 0.6, 0.6]),
        ]
        .spacing(4),
        widget::space::horizontal(),
        button(lucide::shuffle().size(18))
            .on_press_maybe((!detail.albums.is_empty()).then_some(Msg::Shuffle)),
    ]
    .align_y(Center)
    .spacing(16)
    .padding(Padding {
        bottom: 12.0,
        ..Padding::default()
    });

    let body: Element<'_, Msg> = if detail.albums.is_empty() {
        centered_muted("No albums")
    } else {
        let covers = &state.covers;
        let list = lazy_list(&detail.albums, ROW_HEIGHT, move |_index, album: &Album| {
            album_row(covers, album)
        })
        .on_scroll(Msg::Scrolled)
        .width(Fill)
        .height(Fill);

        widget::column![album_header(), list]
            .width(Fill)
            .height(Fill)
            .into()
    };

    widget::column![header, body]
        .width(Fill)
        .height(Fill)
        .into()
}

fn album_header() -> Element<'static, Msg> {
    widget::row![
        widget::container(text("")).width(ROW_COVER_DISPLAY),
        text("Album")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(4)),
        text("Artist")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(3)),
        text("Songs").size(11).color([0.55, 0.55, 0.55]).width(56),
    ]
    .spacing(10)
    .padding(Padding::from([4, 8]))
    .into()
}

fn album_row<'a>(covers: &'a CoverMap, album: &'a Album) -> Element<'a, Msg> {
    widget::row![
        cover_cell(covers, album.cover.clone()),
        button(
            text(&album.name)
                .size(14)
                .wrapping(iced::widget::text::Wrapping::None)
        )
        .on_press(Msg::AlbumClicked(album.id.clone()))
        .width(FillPortion(4)),
        text(album.artist.as_deref().unwrap_or(""))
            .size(12)
            .color([0.65, 0.65, 0.65])
            .wrapping(iced::widget::text::Wrapping::None)
            .width(FillPortion(3)),
        text(album.song_count.to_string()).width(56),
    ]
    .align_y(Center)
    .spacing(10)
    .padding(Padding::from([4, 8]))
    .height(ROW_HEIGHT)
    .width(Fill)
    .into()
}

fn header_cover_view(cover: Option<&CoverState>, size: f32) -> Element<'static, Msg> {
    match cover {
        Some(CoverState::Loaded(handle)) => image(handle.clone())
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

fn cover_cell<'a>(covers: &'a CoverMap, cover: Option<CoverId>) -> Element<'a, Msg> {
    match cover.as_ref().and_then(|id| covers.get(id)) {
        Some(CoverState::Loaded(handle)) => image(handle.clone())
            .filter_method(image::FilterMethod::Linear)
            .width(ROW_COVER_DISPLAY)
            .height(ROW_COVER_DISPLAY)
            .into(),
        _ => widget::container(lucide::music().size(20).color([0.5, 0.5, 0.5]))
            .width(ROW_COVER_DISPLAY)
            .height(ROW_COVER_DISPLAY)
            .align_x(Center)
            .align_y(Center)
            .into(),
    }
}

fn refresh_covers(state: &mut State, manager: &Manager) -> Action<Msg, Event> {
    let Some(detail) = state.load.loaded() else {
        return Action::none();
    };
    let len = detail.albums.len();
    let start = state.visible.start.min(len);
    let end = state.visible.end.min(len).max(start);
    state.visible = start..end;

    let mut wanted: Vec<CoverId> = Vec::new();
    if let Some(cover) = detail.artist.cover.clone() {
        wanted.push(cover);
    }
    wanted.extend(
        detail.albums[start..end]
            .iter()
            .filter_map(|album| album.cover.clone()),
    );

    let task = state
        .covers
        .sync(wanted, HEADER_COVER_PX, &manager.handle())
        .map(|(id, result)| Msg::CoverLoaded(id, result));
    Action::task(task)
}

fn centered_muted(message: impl Into<String>) -> Element<'static, Msg> {
    container(text(message.into()).size(13).color([0.6, 0.6, 0.6]))
        .center(Fill)
        .into()
}
