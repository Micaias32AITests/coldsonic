//! All tracks: a virtualized list of every track in the merged library.
//!
//! The list is windowed by [`lazy_list`], so only visible rows are built. Cover
//! fetching follows the visible window rather than the whole library.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{CoverId, Manager, Track};
use crate::prelude::*;
use crate::widget::lazy_list::lazy_list;

use super::fmt_duration;

const ROW_HEIGHT: f32 = 56.0;
const COVER_DISPLAY: f32 = 48.0;
const COVER_PX: u32 = 96;
const INITIAL_ROWS: usize = 32;
const OVERSCAN: usize = 3;

#[derive(Debug)]
pub struct State {
    rows: Vec<Track>,
    covers: HashMap<CoverId, CoverState>,
    visible: Range<usize>,
    loaded: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum CoverState {
    Loading,
    Loaded(image::Handle),
    Failed,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Loaded(Vec<Track>),
    LoadFailed(String),
    Scrolled(iced::widget::scrollable::Viewport),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    Play(Track),
}

#[derive(Debug, Clone)]
pub enum Event {
    PlayTrack(Track),
}

pub fn init(manager: Arc<Manager>) -> (State, Task<Msg>) {
    (
        State {
            rows: Vec::new(),
            covers: HashMap::new(),
            visible: 0..INITIAL_ROWS,
            loaded: false,
            error: None,
        },
        Task::perform(
            async move { manager.tracks().await },
            |result| match result {
                Ok(tracks) => Msg::Loaded(tracks),
                Err(err) => Msg::LoadFailed(err.to_string()),
            },
        ),
    )
}

pub fn update(state: &mut State, manager: &Arc<Manager>, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Loaded(tracks) => {
            state.rows = tracks;
            state.loaded = true;
            state.error = None;
            state.visible = 0..INITIAL_ROWS.min(state.rows.len());
            refresh_covers(state, manager)
        }
        Msg::LoadFailed(error) => {
            state.rows.clear();
            state.covers.clear();
            state.loaded = true;
            state.error = Some(error);
            Action::none()
        }
        Msg::Scrolled(viewport) => {
            state.visible = visible_range(&viewport);
            refresh_covers(state, manager)
        }
        Msg::CoverLoaded(id, result) => {
            let Some(cover) = state.covers.get_mut(&id) else {
                return Action::none();
            };
            *cover = match result {
                Ok(bytes) => CoverState::Loaded(image::Handle::from_bytes(bytes)),
                Err(_) => CoverState::Failed,
            };
            Action::none()
        }
        Msg::Play(track) => Action::emit(Event::PlayTrack(track)),
    }
}

pub fn view(state: &State) -> Element<'_, Msg> {
    if !state.loaded {
        return centered_muted("loading...");
    }

    if let Some(error) = &state.error {
        return centered_muted(error.clone());
    }

    if state.rows.is_empty() {
        return centered_muted("No tracks found");
    }

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
    let list = lazy_list(&state.rows, ROW_HEIGHT, move |_index, track: &Track| {
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

fn track_row<'a>(covers: &'a HashMap<CoverId, CoverState>, track: &'a Track) -> Element<'a, Msg> {
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

fn cover_cell<'a>(
    covers: &'a HashMap<CoverId, CoverState>,
    cover: Option<&CoverId>,
) -> Element<'a, Msg> {
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

fn refresh_covers(state: &mut State, manager: &Arc<Manager>) -> Action<Msg, Event> {
    let len = state.rows.len();
    let start = state.visible.start.min(len);
    let end = state.visible.end.min(len).max(start);
    state.visible = start..end;

    let mut wanted: HashSet<CoverId> = HashSet::new();
    for track in &state.rows[start..end] {
        if let Some(cover) = &track.cover {
            wanted.insert(cover.clone());
        }
    }

    state.covers.retain(|id, _| wanted.contains(id));

    let mut tasks = Vec::new();
    for id in wanted {
        if state.covers.contains_key(&id) {
            continue;
        }
        state.covers.insert(id.clone(), CoverState::Loading);
        let manager = Arc::clone(manager);
        let fetch = id.clone();
        tasks.push(Task::perform(
            async move { manager.cover(&fetch, COVER_PX).await },
            move |result| Msg::CoverLoaded(id, result.map_err(|_| ())),
        ));
    }

    if tasks.is_empty() {
        Action::none()
    } else {
        Action::task(Task::batch(tasks))
    }
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
