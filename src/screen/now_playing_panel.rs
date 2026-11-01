//! Now-playing panel: a large, details-only view of the current track.
//!
//! Transport, seek, and volume stay in the bottom bar. This screen owns only
//! its cover handle and reads the track from [`Ctx`].

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::CoverId;
use crate::prelude::*;

use super::{Ctx, fmt_duration};

/// Pixel size requested from the library for the large panel cover. Offline
/// providers return the stored art; network providers may downscale.
const COVER_PX: u32 = 2048;

/// Fraction of the panel body width used by the square cover.
const COVER_FRACTION: f32 = 0.8;

/// Body padding kept in sync with the container below, so the cover is sized
/// against the actual content box rather than the outer panel rect.
const BODY_PADDING_X: f32 = 16.0;
const BODY_PADDING_Y: f32 = 12.0;

#[derive(Debug, Default)]
pub struct State {
    cover: Option<image::Handle>,
    /// Cover id `cover` belongs to, for stale-result filtering.
    cover_id: Option<CoverId>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Re-check the borrowed now-playing state for a track/cover change.
    Refresh,
    CoverLoaded(CoverId, Result<Bytes, ()>),
}

/// Details-only: no transport, seek, or volume intents.
#[derive(Debug, Clone)]
pub enum Event {}

pub fn init() -> State {
    State::default()
}

pub fn update(state: &mut State, ctx: Ctx<'_>, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Refresh => refresh(state, ctx),
        Msg::CoverLoaded(id, result) => {
            if state.cover_id.as_ref() != Some(&id) {
                // A newer track is showing; drop the stale result.
                return Action::none();
            }
            state.cover = match result {
                Ok(bytes) => Some(image::Handle::from_bytes(bytes)),
                Err(_) => None,
            };
            Action::none()
        }
    }
}

fn refresh(state: &mut State, ctx: Ctx<'_>) -> Action<Msg, Event> {
    let wanted = ctx.now_playing.and_then(|np| np.track.cover.clone());

    if wanted == state.cover_id {
        return Action::none();
    }

    state.cover = None;
    state.cover_id = wanted.clone();

    let Some(id) = wanted else {
        return Action::none();
    };

    let manager = Arc::clone(ctx.manager);
    let fetch = id.clone();
    Action::task(Task::perform(
        async move { manager.cover(&fetch, COVER_PX).await },
        move |res| Msg::CoverLoaded(id, res.map_err(|_| ())),
    ))
}

pub fn view<'a>(state: &'a State, ctx: Ctx<'a>) -> Element<'a, Msg> {
    let np = ctx.now_playing;

    responsive(move |size| {
        // Size against the padded content box so the cover/icon cannot overflow
        // the panel. The height cap leaves room for the metadata block.
        let available_width = (size.width - BODY_PADDING_X * 2.0).max(0.0);
        let available_height = (size.height - BODY_PADDING_Y * 2.0).max(0.0);
        let side = (available_width * COVER_FRACTION)
            .min(available_height * 0.65)
            .max(0.0);

        let art: Element<'a, Msg> = match (&state.cover, np) {
            (Some(handle), _) => image(handle.clone())
                .filter_method(image::FilterMethod::Linear)
                .width(side)
                .height(side)
                .into(),
            (None, Some(_)) => cover_placeholder(side),
            (None, None) => icon_placeholder((side * 0.45).max(0.0)),
        };

        let body: Element<'a, Msg> = match np {
            Some(np) => widget::column![
                art,
                text(&np.track.title)
                    .size(22)
                    .wrapping(iced::widget::text::Wrapping::None),
                text(np.track.album.as_deref().unwrap_or(""))
                    .size(16)
                    .color([0.7, 0.7, 0.7]),
                text(np.track.artist.as_deref().unwrap_or(""))
                    .size(14)
                    .color([0.6, 0.6, 0.6]),
                text(fmt_duration(np.track.duration))
                    .size(13)
                    .color([0.55, 0.55, 0.55]),
                // future: visualizer
                widget::Space::new(),
            ]
            .align_x(Center)
            .spacing(6)
            .into(),
            None => widget::column![
                art,
                text("No music currently playing")
                    .size(14)
                    .color([0.55, 0.55, 0.55]),
            ]
            .align_x(Center)
            .spacing(12)
            .into(),
        };

        widget::container(body)
            .center(Fill)
            .padding(Padding::from([BODY_PADDING_Y, BODY_PADDING_X]))
            .into()
    })
    .width(Fill)
    .height(Fill)
    .into()
}

/// Square placeholder that reserves the cover's layout space while its bytes
/// are loading.
fn cover_placeholder<'a>(side: f32) -> Element<'a, Msg> {
    let icon_size = (side * 0.25).max(24.0).min(side);
    widget::container(icon_placeholder(icon_size))
        .width(side)
        .height(side)
        .align_x(Center)
        .align_y(Center)
        .style(super::muted_cover_style)
        .into()
}

/// An icon at its natural size. The caller centers it; do not wrap it in a
/// fixed-size square, or the glyph's own metrics can push it off-center.
fn icon_placeholder<'a>(size: f32) -> Element<'a, Msg> {
    lucide::music().size(size).color([0.45, 0.45, 0.45]).into()
}
