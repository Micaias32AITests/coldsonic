//! Albums: every album in the merged library.
//!
//! The screen owns only album rows and cover handles; opening an album is an
//! intent emitted to `MainScreen`.

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Album, AlbumId, CoverId, Manager, ManagerHandle};
use crate::prelude::*;

use super::covers::{CoverMap, CoverState};
use super::load_state::LoadState;

/// Logical size of the cover cell in the table.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2x the display size for HiDPI).
const COVER_PX: u32 = 96;

#[derive(Debug)]
pub struct State {
    load: LoadState<Vec<Album>>,
    covers: CoverMap,
    revision: u64,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded { revision: u64, albums: Vec<Album> },
    LoadFailed { revision: u64, error: String },
    Reload,
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
            let wanted: Vec<CoverId> = albums
                .iter()
                .filter_map(|album| album.cover.clone())
                .collect();
            state.load = LoadState::Loaded(albums);
            let task = state
                .covers
                .sync(wanted, COVER_PX, &manager.handle())
                .map(|(id, result)| Msg::CoverLoaded(id, result));
            Action::task(task)
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
            let list = table(
                [
                    table::column(text(""), |album: Album| {
                        cover_cell(covers, album.cover.clone())
                    }),
                    table::column(text("Album"), |album: Album| {
                        Element::from(
                            button(text(album.name.clone()))
                                .on_press(Msg::AlbumClicked(album.id.clone())),
                        )
                    })
                    .width(Fill),
                    table::column(text("Artist"), |album: Album| {
                        text(album.artist.unwrap_or_default()).color([0.65, 0.65, 0.65])
                    }),
                    table::column(text("Songs"), |album: Album| {
                        text(album.song_count.to_string())
                    }),
                    table::column(text("Duration"), |album: Album| {
                        text(album.duration.map(super::fmt_duration).unwrap_or_default())
                            .color([0.6, 0.6, 0.6])
                    }),
                ],
                albums.clone(),
            )
            .width(Fill);
            scrollable(list).width(Fill).height(Fill).into()
        }
    }
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

fn centered_muted(message: impl Into<String>) -> Element<'static, Msg> {
    container(text(message.into()).size(13).color([0.6, 0.6, 0.6]))
        .center(Fill)
        .into()
}
