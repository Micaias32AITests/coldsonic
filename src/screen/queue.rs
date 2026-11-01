//! Queue panel: the current track pinned above the upcoming queue.
//!
//! Read-only for now. The screen keeps only presentation state (cover handles
//! and the visible range); the queue itself is read live from the engine
//! through [`Ctx`], so there is nothing to synchronize when tracks change.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::time::Duration;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::engine::QueueEntry;
use crate::library::CoverId;
use crate::prelude::*;
use crate::widget::lazy_list::lazy_list;

use super::{Ctx, fmt_duration, muted_cover_style};

/// Fixed row height for the virtualized list and the pinned current row.
const ROW_HEIGHT: f32 = 56.0;

/// Logical cover size in a row.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2× the display size for HiDPI).
const COVER_PX: u32 = 96;

/// Rows built before the first viewport update tells us the real height.
const INITIAL_ROWS: usize = 24;

/// Extra rows fetched above and below the visible window.
const OVERSCAN: usize = 3;

#[derive(Debug, Default)]
pub struct State {
    covers: HashMap<CoverId, CoverState>,
    /// Upcoming entries currently visible to cover prefetch, clamped to the
    /// live queue length on every refresh.
    visible: Range<usize>,
}

#[derive(Debug, Clone)]
enum CoverState {
    Loading,
    Loaded(image::Handle),
    Failed,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Re-scan the current track and visible entries for missing covers.
    Refresh,
    CoverLoaded(CoverId, Result<Bytes, ()>),
    /// Visible range changed; fetch newly visible covers.
    Scrolled(iced::widget::scrollable::Viewport),
}

/// Read-only screen for now; row actions are future work.
#[derive(Debug, Clone)]
pub enum Event {}

pub fn init() -> State {
    State {
        covers: HashMap::new(),
        visible: 0..INITIAL_ROWS,
    }
}

pub fn update(state: &mut State, ctx: Ctx<'_>, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Refresh => refresh(state, ctx),
        Msg::Scrolled(viewport) => {
            state.visible = visible_range(&viewport);
            refresh(state, ctx)
        }
        Msg::CoverLoaded(id, result) => {
            // Ignore results for covers pruned out of the current window.
            let Some(cover) = state.covers.get_mut(&id) else {
                return Action::none();
            };
            *cover = match result {
                Ok(bytes) => CoverState::Loaded(image::Handle::from_bytes(bytes)),
                Err(_) => CoverState::Failed,
            };
            Action::none()
        }
    }
}

fn refresh(state: &mut State, ctx: Ctx<'_>) -> Action<Msg, Event> {
    let len = ctx.engine.queue().len();
    let start = state.visible.start.min(len);
    let end = state.visible.end.min(len).max(start);
    state.visible = start..end;

    let mut wanted: HashSet<CoverId> = HashSet::new();
    if let Some(cover) = ctx.now_playing.and_then(|np| np.track.cover.clone()) {
        wanted.insert(cover);
    }

    let entries: Vec<&QueueEntry> = ctx
        .engine
        .queue()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect();
    for entry in &entries {
        if let Some(cover) = entry.track.cover.clone() {
            wanted.insert(cover);
        }
    }

    // Keep the cache bounded to the current window. Failed entries are kept
    // until they scroll out, so a broken cover is not re-fetched every tick.
    state.covers.retain(|id, _| wanted.contains(id));

    let mut tasks = Vec::new();
    for id in wanted {
        if state.covers.contains_key(&id) {
            continue;
        }
        state.covers.insert(id.clone(), CoverState::Loading);
        let manager = Arc::clone(ctx.manager);
        let fetch = id.clone();
        tasks.push(Task::perform(
            async move { manager.cover(&fetch, COVER_PX).await },
            move |res| Msg::CoverLoaded(id, res.map_err(|_| ())),
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

pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg> {
    let np = ctx.now_playing;
    let has_upcoming = ctx.engine.queue().len() > 0;

    if np.is_none() && !has_upcoming {
        return empty_state("Nothing playing");
    }

    let mut content = widget::column![].spacing(0);

    if let Some(np) = np {
        content = content.push(section_header("NOW PLAYING"));
        content = content.push(current_row(
            &state.covers,
            np.track.cover.as_ref(),
            &np.track.title,
            np.track.artist.as_deref(),
            np.track.duration,
        ));
        content = content
            .push(widget::container(widget::rule::horizontal(1)).padding(Padding::from([8, 0])));
    }

    let upcoming_count = ctx.engine.queue().len();
    content = content.push(
        widget::row![
            section_header("UP NEXT"),
            widget::space::horizontal(),
            text(upcoming_count.to_string())
                .size(11)
                .color([0.55, 0.55, 0.55]),
        ]
        .align_y(Center)
        .padding(Padding::from([4, 0])),
    );

    if !has_upcoming {
        content = content.push(
            widget::container(text("Nothing up next").size(13).color([0.55, 0.55, 0.55]))
                .padding(Padding::from([12, 8])),
        );
    } else {
        let covers = &state.covers;
        let list = lazy_list(ctx.engine.queue(), ROW_HEIGHT, move |_index, entry| {
            upcoming_row(
                covers,
                entry.track.cover.as_ref(),
                &entry.track.title,
                entry.track.artist.as_deref(),
                entry.track.duration,
            )
        })
        .on_scroll(Msg::Scrolled)
        .width(Fill)
        .height(Fill);

        content = content.push(list);
    }

    content.width(Fill).height(Fill).into()
}

fn section_header<'a>(label: &'a str) -> Element<'a, Msg> {
    text(label).size(11).color([0.55, 0.55, 0.55]).into()
}

fn current_row<'a>(
    covers: &'a HashMap<CoverId, CoverState>,
    cover: Option<&'a CoverId>,
    title: &'a str,
    artist: Option<&'a str>,
    duration: Duration,
) -> Element<'a, Msg> {
    widget::container(track_row(covers, cover, title, artist, duration))
        .width(Fill)
        .height(ROW_HEIGHT)
        .style(current_row_style)
        .into()
}

fn upcoming_row<'a>(
    covers: &'a HashMap<CoverId, CoverState>,
    cover: Option<&'a CoverId>,
    title: &'a str,
    artist: Option<&'a str>,
    duration: Duration,
) -> Element<'a, Msg> {
    widget::container(track_row(covers, cover, title, artist, duration))
        .width(Fill)
        .height(ROW_HEIGHT)
        .clip(true)
        .into()
}

fn track_row<'a>(
    covers: &'a HashMap<CoverId, CoverState>,
    cover: Option<&'a CoverId>,
    title: &'a str,
    artist: Option<&'a str>,
    duration: Duration,
) -> Element<'a, Msg> {
    let info = widget::column![
        text(title)
            .size(14)
            .wrapping(iced::widget::text::Wrapping::None)
            .width(Fill),
        text(artist.unwrap_or(""))
            .size(12)
            .color([0.6, 0.6, 0.6])
            .wrapping(iced::widget::text::Wrapping::None)
            .width(Fill),
    ]
    .spacing(2)
    .width(Fill);

    widget::row![
        cover_cell(covers, cover),
        info,
        text(fmt_duration(duration)).size(12).color([0.6, 0.6, 0.6]),
    ]
    .align_y(Center)
    .spacing(10)
    .padding(Padding::from([4, 8]))
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
            .style(muted_cover_style)
            .into(),
    }
}

fn empty_state(message: &str) -> Element<'_, Msg> {
    widget::container(
        widget::column![
            lucide::music().size(48).color([0.45, 0.45, 0.45]),
            text(message).size(13).color([0.55, 0.55, 0.55]),
        ]
        .align_x(Center)
        .spacing(12),
    )
    .center(Fill)
    .into()
}

fn current_row_style(theme: &Theme) -> widget::container::Style {
    widget::container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        ..Default::default()
    }
}
