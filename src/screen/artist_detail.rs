//! Artist detail: the selected artist's header and albums.
//!
//! The screen owns only local presentation state (the loaded artist, album
//! rows, and cover handles). Navigation and playback are intents emitted to
//! `MainScreen`.

use std::collections::HashSet;

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Album, AlbumId, Artist, ArtistDetail, ArtistId, CoverId, Manager};
use crate::prelude::*;

/// Pixel size requested for the header cover (2x the 128 px display).
const HEADER_COVER_PX: u32 = 256;
const HEADER_COVER_DISPLAY: f32 = 128.0;

/// Pixel size requested for row covers (2x the 48 px display).
const ROW_COVER_PX: u32 = 96;
const ROW_COVER_DISPLAY: f32 = 48.0;

#[derive(Debug)]
pub struct State {
    id: ArtistId,
    artist: Option<Artist>,
    albums: Vec<AlbumRow>,
    loaded: bool,
    error: Option<String>,
    cover: CoverState,
}

#[derive(Debug, Clone)]
struct AlbumRow {
    album: Album,
    cover: CoverState,
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
    Loaded(ArtistId, Result<ArtistDetail, String>),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    AlbumClicked(AlbumId),
    Shuffle,
}

#[derive(Debug, Clone)]
pub enum Event {
    AlbumClicked(AlbumId),
    PlayShuffled(ArtistId),
}

pub fn init(id: ArtistId, manager: Arc<Manager>) -> (State, Task<Msg>) {
    let fetch_id = id.clone();
    let message_id = id.clone();
    let task = Task::perform(
        async move {
            manager
                .artist(&fetch_id)
                .await
                .map_err(|err| err.to_string())
        },
        move |result| Msg::Loaded(message_id, result),
    );

    (
        State {
            id,
            artist: None,
            albums: Vec::new(),
            loaded: false,
            error: None,
            cover: CoverState::NotLoaded,
        },
        task,
    )
}

pub fn update(state: &mut State, manager: &Arc<Manager>, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Loaded(id, result) => {
            if state.id != id {
                return Action::none();
            }

            state.loaded = true;
            match result {
                Ok(detail) => {
                    state.artist = Some(detail.artist);
                    state.albums = Vec::with_capacity(detail.albums.len());
                    state.error = None;

                    let mut tasks = Vec::new();
                    let mut seen = HashSet::new();

                    if let Some(cover) = state
                        .artist
                        .as_ref()
                        .and_then(|artist| artist.cover.clone())
                    {
                        seen.insert(cover.clone());
                        state.cover = CoverState::Loading;
                        tasks.push(cover_task(manager, cover, HEADER_COVER_PX));
                    }

                    for album in detail.albums {
                        let cover = album.cover.clone();
                        let cover_state = if let Some(id) = &cover {
                            if seen.insert(id.clone()) {
                                tasks.push(cover_task(manager, id.clone(), ROW_COVER_PX));
                            }
                            CoverState::Loading
                        } else {
                            CoverState::NotLoaded
                        };
                        state.albums.push(AlbumRow {
                            album,
                            cover: cover_state,
                        });
                    }

                    tasks_to_action(tasks)
                }
                Err(error) => {
                    state.artist = None;
                    state.albums.clear();
                    state.cover = CoverState::NotLoaded;
                    state.error = Some(error);
                    Action::none()
                }
            }
        }
        Msg::CoverLoaded(id, result) => {
            // The header and one or more album rows may share a cover id, and
            // the cover fetch is deduplicated, so apply the result everywhere
            // it is used rather than stopping at the first match.
            let cover = cover_from_result(result);

            if state
                .artist
                .as_ref()
                .and_then(|artist| artist.cover.as_ref())
                == Some(&id)
            {
                state.cover = cover.clone();
            }

            for row in &mut state.albums {
                if row.album.cover.as_ref() == Some(&id) {
                    row.cover = cover.clone();
                }
            }
            Action::none()
        }
        Msg::AlbumClicked(id) => Action::emit(Event::AlbumClicked(id)),
        Msg::Shuffle => Action::emit(Event::PlayShuffled(state.id.clone())),
    }
}

pub fn view(state: &State) -> Element<'_, Msg> {
    if !state.loaded {
        return centered_muted("loading...");
    }

    if let Some(error) = &state.error {
        return centered_muted(error.clone());
    }

    let Some(artist) = &state.artist else {
        return centered_muted("Artist not found");
    };

    let header = widget::row![
        cover_view(&state.cover, HEADER_COVER_DISPLAY),
        widget::column![
            text(&artist.name).size(24),
            text(format!("{} albums", artist.album_count))
                .size(12)
                .color([0.6, 0.6, 0.6]),
        ]
        .spacing(4),
        widget::space::horizontal(),
        button(lucide::shuffle().size(18))
            .on_press_maybe((!state.albums.is_empty()).then_some(Msg::Shuffle)),
    ]
    .align_y(Center)
    .spacing(16)
    .padding(Padding {
        bottom: 12.0,
        ..Padding::default()
    });

    let body: Element<'_, Msg> = if state.albums.is_empty() {
        centered_muted("No albums")
    } else {
        let list = table(
            [
                table::column(text(""), |row: AlbumRow| cover_cell(row)),
                table::column(text("Album"), |row: AlbumRow| {
                    Element::from(
                        button(text(row.album.name.clone()))
                            .on_press(Msg::AlbumClicked(row.album.id.clone())),
                    )
                })
                .width(Fill),
                table::column(text("Artist"), |row: AlbumRow| {
                    text(row.album.artist.unwrap_or_default()).color([0.65, 0.65, 0.65])
                }),
                table::column(text("Songs"), |row: AlbumRow| {
                    text(row.album.song_count.to_string())
                }),
            ],
            state.albums.clone(),
        )
        .width(Fill);

        scrollable(list).width(Fill).height(Fill).into()
    };

    widget::column![header, body]
        .width(Fill)
        .height(Fill)
        .into()
}

fn cover_task(manager: &Arc<Manager>, id: CoverId, px: u32) -> Task<Msg> {
    let manager = Arc::clone(manager);
    let returned_id = id.clone();
    Task::perform(async move { manager.cover(&id, px).await }, move |result| {
        Msg::CoverLoaded(returned_id, result.map_err(|_| ()))
    })
}

fn tasks_to_action(tasks: Vec<Task<Msg>>) -> Action<Msg, Event> {
    if tasks.is_empty() {
        Action::none()
    } else {
        Action::task(Task::batch(tasks))
    }
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

fn cover_cell(row: AlbumRow) -> Element<'static, Msg> {
    match row.cover {
        CoverState::Loaded(handle) => image(handle)
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

fn centered_muted(message: impl Into<String>) -> Element<'static, Msg> {
    container(text(message.into()).size(13).color([0.6, 0.6, 0.6]))
        .center(Fill)
        .into()
}
