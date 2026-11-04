//! All tracks: a virtualized list of every track in the merged library.
//!
//! The list is windowed by [`lazy_list`], so only visible rows are built. Cover
//! fetching follows the visible window rather than the whole library.

use std::ops::Range;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{CoverId, Manager, ManagerHandle, Track};
use crate::prelude::*;
use crate::widget::lazy_list::lazy_list;

use super::covers::{CoverMap, CoverState};
use super::fmt_duration;
use super::load_state::LoadState;

const ROW_HEIGHT: f32 = 56.0;
const COVER_DISPLAY: f32 = 48.0;
const COVER_PX: u32 = 96;
const INITIAL_ROWS: usize = 32;
const OVERSCAN: usize = 3;

#[derive(Debug)]
pub struct State {
    load: LoadState<Vec<Track>>,
    covers: CoverMap,
    visible: Range<usize>,
    revision: u64,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded { revision: u64, tracks: Vec<Track> },
    LoadFailed { revision: u64, error: String },
    Reload,
    Scrolled(iced::widget::scrollable::Viewport),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    Play(Track),
}

#[derive(Debug, Clone)]
pub enum Event {
    PlayTrack(Track),
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
        async move { handle.tracks().await },
        move |result| match result {
            Ok(tracks) => Msg::Loaded { revision, tracks },
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
        Msg::Loaded { revision, tracks } => {
            if revision != state.revision {
                return Action::none();
            }
            state.dirty = false;
            state.covers.clear();
            state.load = LoadState::Loaded(tracks);
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
            state.visible = visible_range(&viewport);
            refresh_covers(state, manager)
        }
        Msg::CoverLoaded(id, result) => {
            state.covers.apply(id, result);
            Action::none()
        }
        Msg::Play(track) => Action::emit(Event::PlayTrack(track)),
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
    let tracks = match &state.load {
        LoadState::Loading => return centered_muted("loading..."),
        LoadState::Error(error) => return centered_muted(error.clone()),
        LoadState::NotFound => return centered_muted("No tracks found"),
        LoadState::Loaded(tracks) if tracks.is_empty() => return centered_muted("No tracks found"),
        LoadState::Loaded(tracks) => tracks,
    };

    let header = widget::row![
        widget::container(text("")).width(COVER_DISPLAY),
        text("Title")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(4)),
        text("Artist")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(3)),
        text("Album")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(FillPortion(3)),
        text("Duration")
            .size(11)
            .color([0.55, 0.55, 0.55])
            .width(56),
        widget::container(text("")).width(32),
    ]
    .spacing(10)
    .padding(Padding::from([4, 8]));

    let covers = &state.covers;
    let list = lazy_list(tracks, ROW_HEIGHT, move |_index, track: &Track| {
        track_row(covers, track)
    })
    .on_scroll(Msg::Scrolled)
    .width(Fill)
    .height(Fill);

    widget::column![header, list]
        .width(Fill)
        .height(Fill)
        .into()
}

fn track_row<'a>(covers: &'a CoverMap, track: &'a Track) -> Element<'a, Msg> {
    widget::row![
        cover_cell(covers, track.cover.as_ref()),
        text(&track.title)
            .size(14)
            .wrapping(iced::widget::text::Wrapping::None)
            .width(FillPortion(4)),
        text(track.artist.as_deref().unwrap_or(""))
            .size(12)
            .color([0.65, 0.65, 0.65])
            .wrapping(iced::widget::text::Wrapping::None)
            .width(FillPortion(3)),
        text(track.album.as_deref().unwrap_or(""))
            .size(12)
            .color([0.65, 0.65, 0.65])
            .wrapping(iced::widget::text::Wrapping::None)
            .width(FillPortion(3)),
        text(fmt_duration(track.duration))
            .size(12)
            .color([0.6, 0.6, 0.6])
            .width(56),
        button(lucide::play().size(14))
            .on_press(Msg::Play(track.clone()))
            .padding(4),
    ]
    .align_y(Center)
    .spacing(10)
    .padding(Padding::from([4, 8]))
    .height(ROW_HEIGHT)
    .width(Fill)
    .into()
}

fn cover_cell<'a>(covers: &'a CoverMap, cover: Option<&'a CoverId>) -> Element<'a, Msg> {
    match cover.and_then(|id| covers.get(id)) {
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
    let Some(tracks) = state.load.loaded() else {
        return Action::none();
    };
    let len = tracks.len();
    let start = state.visible.start.min(len);
    let end = state.visible.end.min(len).max(start);
    state.visible = start..end;

    let wanted: Vec<CoverId> = tracks[start..end]
        .iter()
        .filter_map(|track| track.cover.clone())
        .collect();

    let task = state
        .covers
        .sync(wanted, COVER_PX, &manager.handle())
        .map(|(id, result)| Msg::CoverLoaded(id, result));
    Action::task(task)
}

fn visible_range(viewport: &iced::widget::scrollable::Viewport) -> Range<usize> {
    let offset = viewport.absolute_offset().y.max(0.0);
    let height = viewport.bounds().height.max(0.0);

    let first = (offset / ROW_HEIGHT).floor() as usize;
    let last = ((offset + height) / ROW_HEIGHT).ceil() as usize;

    first.saturating_sub(OVERSCAN)..last.saturating_add(OVERSCAN).max(first + 1)
}

fn centered_muted(message: impl Into<String>) -> Element<'static, Msg> {
    container(text(message.into()).size(13).color([0.6, 0.6, 0.6]))
        .center(Fill)
        .into()
}
