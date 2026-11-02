//! Albums: every album in the merged library.
//!
//! The screen owns only album rows and cover handles; opening an album is an
//! intent emitted to `MainScreen`.

use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Album, AlbumId, CoverId, Manager};
use crate::prelude::*;

/// Logical size of the cover cell in the table.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2x the display size for HiDPI).
const COVER_PX: u32 = 96;

#[derive(Debug)]
pub struct State {
    rows: Vec<Row>,
    loaded: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct Row {
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
    Loaded(Vec<Album>),
    LoadFailed(String),
    AlbumClicked(AlbumId),
    CoverLoaded(CoverId, Result<Bytes, ()>),
}

#[derive(Debug, Clone)]
pub enum Event {
    AlbumClicked(AlbumId),
}

pub fn init(manager: Arc<Manager>) -> (State, Task<Msg>) {
    (
        State {
            rows: Vec::new(),
            loaded: false,
            error: None,
        },
        Task::perform(
            async move { manager.albums().await },
            |result| match result {
                Ok(albums) => Msg::Loaded(albums),
                Err(err) => Msg::LoadFailed(err.to_string()),
            },
        ),
    )
}

pub fn update(state: &mut State, manager: &Arc<Manager>, msg: Msg) -> Action<Msg, Event> {
    match msg {
        Msg::Loaded(albums) => {
            let mut rows = Vec::with_capacity(albums.len());
            let mut tasks = Vec::new();

            for album in albums {
                let cover = if let Some(id) = album.cover.clone() {
                    tasks.push(cover_task(manager, id));
                    CoverState::Loading
                } else {
                    CoverState::NotLoaded
                };
                rows.push(Row { album, cover });
            }

            state.rows = rows;
            state.loaded = true;
            state.error = None;
            tasks_to_action(tasks)
        }
        Msg::LoadFailed(error) => {
            state.rows.clear();
            state.loaded = true;
            state.error = Some(error);
            Action::none()
        }
        Msg::AlbumClicked(id) => Action::emit(Event::AlbumClicked(id)),
        Msg::CoverLoaded(id, result) => {
            if let Some(row) = state
                .rows
                .iter_mut()
                .find(|row| row.album.cover.as_ref() == Some(&id))
            {
                row.cover = cover_from_result(result);
            }
            Action::none()
        }
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
        return centered_muted("No albums found");
    }

    let list = table(
        [
            table::column(text(""), |row: Row| cover_cell(row)),
            table::column(text("Album"), |row: Row| {
                Element::from(
                    button(text(row.album.name.clone()))
                        .on_press(Msg::AlbumClicked(row.album.id.clone())),
                )
            })
            .width(Fill),
            table::column(text("Artist"), |row: Row| {
                text(row.album.artist.unwrap_or_default()).color([0.65, 0.65, 0.65])
            }),
            table::column(text("Songs"), |row: Row| {
                text(row.album.song_count.to_string())
            }),
            table::column(text("Duration"), |row: Row| {
                text(
                    row.album
                        .duration
                        .map(super::fmt_duration)
                        .unwrap_or_default(),
                )
                .color([0.6, 0.6, 0.6])
            }),
        ],
        state.rows.clone(),
    )
    .width(Fill);

    scrollable(list).width(Fill).height(Fill).into()
}

fn cover_task(manager: &Arc<Manager>, id: CoverId) -> Task<Msg> {
    let manager = Arc::clone(manager);
    let returned_id = id.clone();
    Task::perform(
        async move { manager.cover(&id, COVER_PX).await },
        move |result| Msg::CoverLoaded(returned_id, result.map_err(|_| ())),
    )
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

fn cover_cell(row: Row) -> Element<'static, Msg> {
    match row.cover {
        CoverState::Loaded(handle) => image(handle)
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
