//! Artists: every artist in the merged library.
//!
//! The list is windowed by [`lazy_list`]: only visible rows are built and
//! shaped. A non-virtualized table made the first frame after loading take
//! seconds on large libraries, which looked like the UI was stuck on
//! `loading...`.

use std::ops::Range;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Artist, ArtistId, CoverId, Manager, ManagerHandle};
use crate::prelude::*;
use crate::widget::lazy_list::{lazy_list, visible_range};
use crate::widget::text_button::text_button;

use super::covers::{CoverMap, CoverState};
use super::load_state::LoadState;

#[derive(Debug)]
pub struct State {
    load: LoadState<Vec<Artist>>,
    covers: CoverMap,
    visible: Range<usize>,
    revision: u64,
    dirty: bool,
}

/// Logical size of the cover cell in the table.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2x the display size for HiDPI).
const COVER_PX: u32 = 96;

const ROW_HEIGHT: f32 = 56.0;
const INITIAL_ROWS: usize = 32;
const OVERSCAN: usize = 3;

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded { revision: u64, artists: Vec<Artist> },
    LoadFailed { revision: u64, error: String },
    Reload,
    Scrolled(iced::widget::scrollable::Viewport),
    ArtistClicked(ArtistId),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    PlayShuffled(ArtistId),
}

/// Events this screen raises to its parent (`MainScreen`, the coordinator).
#[derive(Debug, Clone)]
pub enum Event {
    ArtistClicked(ArtistId),
    PlayShuffled(ArtistId),
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
        async move { handle.artists().await },
        move |result| match result {
            Ok(artists) => Msg::Loaded { revision, artists },
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
        Msg::Loaded { revision, artists } => {
            if revision != state.revision {
                return Action::none();
            }
            state.dirty = false;
            state.covers.clear();
            state.load = LoadState::Loaded(artists);
            state.visible = 0..INITIAL_ROWS;
            refresh_covers(state, manager)
        }
        Msg::LoadFailed { revision, error } => {
            if revision != state.revision {
                return Action::none();
            }
            warn!(%error, "failed to load artists");
            state.dirty = false;
            state.covers.clear();
            state.load = LoadState::Error(error);
            Action::none()
        }
        Msg::Scrolled(viewport) => {
            state.visible = visible_range(&viewport, ROW_HEIGHT, OVERSCAN);
            refresh_covers(state, manager)
        }
        Msg::ArtistClicked(id) => {
            debug!(id = %id.as_str(), "artist clicked");
            Action::emit(Event::ArtistClicked(id))
        }
        Msg::CoverLoaded(id, result) => {
            state.covers.apply(id, result);
            Action::none()
        }
        Msg::PlayShuffled(id) => Action::emit(Event::PlayShuffled(id)),
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

pub fn view(state: &State) -> iced::Element<'_, Msg> {
    match &state.load {
        LoadState::Loading => text("loading...").into(),
        LoadState::Error(error) => centered_muted(error.clone()),
        LoadState::NotFound => centered_muted("No artists found"),
        LoadState::Loaded(artists) if artists.is_empty() => centered_muted("No artists found"),
        LoadState::Loaded(artists) => {
            let covers = &state.covers;
            let list = lazy_list(artists, ROW_HEIGHT, move |_index, artist: &Artist| {
                artist_row(covers, artist)
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
        text("Artist")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(4)),
        text("Albums").size(11).color([0.55, 0.55, 0.55]).width(80),
        widget::container(text("")).width(32),
    ]
    .spacing(10)
    .padding(Padding::from([4, 8]))
    .into()
}

fn artist_row<'a>(covers: &'a CoverMap, artist: &'a Artist) -> Element<'a, Msg> {
    widget::row![
        cover_cell(covers, artist.cover.clone()),
        text_button(
            text(&artist.name)
                .size(14)
                .wrapping(iced::widget::text::Wrapping::None)
        )
        .on_press(Msg::ArtistClicked(artist.id.clone()))
        .width(FillPortion(4)),
        text(artist.album_count.to_string()).width(80),
        shuffle_button(artist),
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
        Some(CoverState::Loading) => text("…").into(),
        _ => widget::container(lucide::music().size(20).color([0.5, 0.5, 0.5]))
            .width(COVER_DISPLAY)
            .height(COVER_DISPLAY)
            .align_x(Center)
            .align_y(Center)
            .into(),
    }
}

fn shuffle_button(artist: &Artist) -> Element<'static, Msg> {
    button(lucide::shuffle().size(16))
        .on_press(Msg::PlayShuffled(artist.id.clone()))
        .into()
}

fn refresh_covers(state: &mut State, manager: &Manager) -> Action<Msg, Event> {
    let Some(artists) = state.load.loaded() else {
        return Action::none();
    };
    let len = artists.len();
    let start = state.visible.start.min(len);
    let end = state.visible.end.min(len).max(start);
    state.visible = start..end;

    let wanted: Vec<CoverId> = artists[start..end]
        .iter()
        .filter_map(|artist| artist.cover.clone())
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
