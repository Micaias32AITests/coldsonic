//! Albums: every album in the merged library.
//!
//! The screen owns only album rows and cover handles; opening an album is an
//! intent emitted to `MainScreen`. The list is virtualized so a large library
//! does not shape every row on the UI thread before showing the first frame.

use std::ops::Range;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Album, AlbumId, CoverId, Manager, ManagerHandle};
use crate::prelude::*;
use crate::widget::lazy_list::{lazy_list, visible_range};

use super::covers::{CoverMap, CoverState};
use super::load_state::LoadState;

/// Logical size of the cover cell in the table.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2x the display size for HiDPI).
const COVER_PX: u32 = 96;

const ROW_HEIGHT: f32 = 56.0;
const INITIAL_ROWS: usize = 32;
const OVERSCAN: usize = 3;

#[derive(Debug)]
pub struct State {
    load: LoadState<Vec<Album>>,
    covers: CoverMap,
    visible: Range<usize>,
    revision: u64,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded { revision: u64, albums: Vec<Album> },
    LoadFailed { revision: u64, error: String },
    Reload,
    Scrolled(iced::widget::scrollable::Viewport),
    AlbumClicked(AlbumId),
    CoverLoaded(CoverId, Result<Bytes, ()>),
}

#[derive(Debug, Clone)]
pub enum Event {
    AlbumClicked(AlbumId),
}

pub fn init(handle: ManagerHandle, revision: u64) -> (State, Task<Msg>) {
    (
        State {
            load: LoadState::Loading,
            covers: CoverMap::default(),
            visible: 0..INITIAL_ROWS,
            revision,
            dirty: false,
        },
        fetch(handle, revision),
    )
}

fn fetch(handle: ManagerHandle, revision: u64) -> Task<Msg> {
    Task::perform(
        async move { handle.albums().await },
        move |result| match result {
            Ok(albums) => Msg::Loaded { revision, albums },
            Err(err) => Msg::LoadFailed {
                revision,
                error: err.to_string(),
            },
        },
    )
}

pub fn update(state: &mut State, manager: &Manager, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Reload => {
            state.dirty = true;
            Action::task(fetch(manager.handle(), state.revision))
        }
        Msg::Loaded { revision, albums } => {
            if revision != state.revision {
                return Action::none();
            }
            state.dirty = false;
            state.covers.clear();
            state.load = LoadState::Loaded(albums);
            state.visible = 0..INITIAL_ROWS;
            refresh_covers(state, manager)
        }
        Msg::LoadFailed { revision, error } => {
            if revision != state.revision {
                return Action::none();
            }
            state.dirty = false;
            state.covers.clear();
            state.load = LoadState::Error(error);
            Action::none()
        }
        Msg::Scrolled(viewport) => {
            state.visible = visible_range(&viewport, ROW_HEIGHT, OVERSCAN);
            refresh_covers(state, manager)
        }
        Msg::AlbumClicked(id) => Action::emit(Event::AlbumClicked(id)),
        Msg::CoverLoaded(id, result) => {
            state.covers.apply(id, result);
            Action::none()
        }
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
    match &state.load {
        LoadState::Loading => centered_muted("loading..."),
        LoadState::Error(error) => centered_muted(error.clone()),
        LoadState::NotFound => centered_muted("No albums found"),
        LoadState::Loaded(albums) if albums.is_empty() => centered_muted("No albums found"),
        LoadState::Loaded(albums) => {
            let covers = &state.covers;
            let list = lazy_list(albums, ROW_HEIGHT, move |_index, album: &Album| {
                album_row(covers, album)
            })
            .on_scroll(Msg::Scrolled)
            .width(Fill)
            .height(Fill);

            widget::column![header(), list]
                .width(Fill)
                .height(Fill)
                .into()
        }
    }
}

fn header() -> Element<'static, Msg> {
    widget::row![
        widget::container(text("")).width(COVER_DISPLAY),
        text("Album")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(4)),
        text("Artist")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(3)),
        text("Songs").size(11).color([0.55, 0.55, 0.55]).width(56),
        text("Duration")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(80),
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
        text(album.duration.map(super::fmt_duration).unwrap_or_default())
            .size(12)
            .color([0.6, 0.6, 0.6])
            .width(80),
    ]
    .align_y(Center)
    .spacing(10)
    .padding(Padding::from([4, 8]))
    .height(ROW_HEIGHT)
    .width(Fill)
    .into()
}

fn cover_cell<'a>(covers: &'a CoverMap, cover: Option<CoverId>) -> Element<'a, Msg> {
    match cover.as_ref().and_then(|id| covers.get(id)) {
        Some(CoverState::Loaded(handle)) => image(handle.clone())
            .filter_method(image::FilterMethod::Linear)
            .width(COVER_DISPLAY)
            .height(COVER_DISPLAY)
            .into(),
        _ => widget::container(lucide::music().size(20).color([0.5, 0.5, 0.5]))
            .width(COVER_DISPLAY)
            .height(COVER_DISPLAY)
            .align_x(Center)
            .align_y(Center)
            .into(),
    }
}

fn refresh_covers(state: &mut State, manager: &Manager) -> Action<Msg, Event> {
    let Some(albums) = state.load.loaded() else {
        return Action::none();
    };
    let len = albums.len();
    let start = state.visible.start.min(len);
    let end = state.visible.end.min(len).max(start);
    state.visible = start..end;

    let wanted: Vec<CoverId> = albums[start..end]
        .iter()
        .filter_map(|album| album.cover.clone())
        .collect();

    let task = state
        .covers
        .sync(wanted, COVER_PX, &manager.handle())
        .map(|(id, result)| Msg::CoverLoaded(id, result));
    Action::task(task)
}

fn centered_muted(message: impl Into<String>) -> Element<'static, Msg> {
    container(text(message.into()).size(13).color([0.6, 0.6, 0.6]))
        .center(Fill)
        .into()
}
