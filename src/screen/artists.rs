//! Artists: every artist in the merged library.

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Artist, ArtistId, CoverId, Manager, ManagerHandle};
use crate::prelude::*;

use super::covers::{CoverMap, CoverState};
use super::load_state::LoadState;

#[derive(Debug)]
pub struct State {
    load: LoadState<Vec<Artist>>,
    covers: CoverMap,
    revision: u64,
    dirty: bool,
}

/// Logical size of the cover cell in the table.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2x the display size for HiDPI).
const COVER_PX: u32 = 96;

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded { revision: u64, artists: Vec<Artist> },
    LoadFailed { revision: u64, error: String },
    Reload,
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
            let wanted: Vec<CoverId> = artists
                .iter()
                .filter_map(|artist| artist.cover.clone())
                .collect();
            state.load = LoadState::Loaded(artists);
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
            warn!(%error, "failed to load artists");
            state.dirty = false;
            state.covers.clear();
            state.load = LoadState::Error(error);
            Action::none()
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
            let list = table(
                [
                    table::column(text(""), |artist: Artist| {
                        cover_cell(covers, artist.cover.clone())
                    }),
                    table::column(text("Artist"), |artist: Artist| {
                        Element::from(
                            button(text(artist.name.clone()))
                                .on_press(Msg::ArtistClicked(artist.id.clone())),
                        )
                    })
                    .width(Fill),
                    table::column(text("Albums"), |artist: Artist| {
                        text(artist.album_count.to_string())
                    }),
                    table::column(text(""), |artist: Artist| shuffle_button(artist)),
                ],
                artists.clone(),
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
        Some(CoverState::Loading) => text("…").into(),
        _ => widget::container(lucide::music().size(20).color([0.5, 0.5, 0.5]))
            .width(COVER_DISPLAY)
            .height(COVER_DISPLAY)
            .align_x(Center)
            .align_y(Center)
            .into(),
    }
}

fn shuffle_button(artist: Artist) -> Element<'static, Msg> {
    button(lucide::shuffle().size(16))
        .on_press(Msg::PlayShuffled(artist.id))
        .into()
}

fn centered_muted(message: impl Into<String>) -> Element<'static, Msg> {
    container(text(message.into()).size(13).color([0.6, 0.6, 0.6]))
        .center(Fill)
        .into()
}
