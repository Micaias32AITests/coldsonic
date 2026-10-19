use bytes::Bytes;
use iced_fonts::lucide;

use crate::library::{Artist, ArtistId, CoverId, Manager};
use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct State {
    manager: Arc<Manager>,
    rows: Vec<Row>,
    /// Distinguishes "still loading" from "loaded, nothing here".
    loaded: bool,
}

#[derive(Debug, Clone)]
struct Row {
    artist: Artist,
    cover: CoverState,
}

/// Logical size of the cover cell in the table.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2x the display size for HiDPI;
/// the offline store returns art as stored, a networked provider downsizes).
const COVER_PX: u32 = 96;

#[derive(Debug, Clone)]
enum CoverState {
    NotLoaded,
    Loading,
    /// The handle is built **once** when the bytes arrive and reused on every
    /// frame. Building it in `view()` would create a fresh unique `Id` each
    /// frame, which defeats iced's image cache (the cover would be re-decoded
    /// and re-uploaded to the GPU every frame).
    Loaded(image::Handle),
    Failed,
}

#[derive(Debug, Clone)]
pub enum Msg {
    ArtistsLoaded(Vec<Artist>),
    ArtistsLoadFailed,
    ArtistClicked(ArtistId),
    CoverLoaded(CoverId, Result<Bytes, ()>),
    PlayShuffled(ArtistId),
}

/// Events this screen raises to its parent (`MainScreen`, the coordinator).
#[derive(Debug, Clone)]
pub enum Event {
    PlayShuffled(ArtistId),
}

impl Screen for State {
    type Msg = Msg;

    type Emit = Event;

    type InitData = Arc<Manager>;

    fn init(manager: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        let manager_for_load = manager.clone();
        (
            Self {
                manager,
                rows: Vec::new(),
                loaded: false,
            },
            Action::task(Task::perform(
                async move { manager_for_load.artists().await },
                |res| match res {
                    Ok(artists) => Msg::ArtistsLoaded(artists),
                    Err(_) => Msg::ArtistsLoadFailed,
                },
            )),
        )
    }

    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit> {
        match msg {
            Msg::ArtistsLoaded(artists) => {
                let mut rows = Vec::with_capacity(artists.len());
                let mut tasks = Vec::new();
                for artist in artists.into_iter() {
                    if let Some(cover) = artist.cover.clone() {
                        let manager = self.manager.clone();
                        let fetch_cover = cover.clone();
                        tasks.push(Task::perform(
                            async move { manager.cover(&fetch_cover, COVER_PX).await },
                            move |res| Msg::CoverLoaded(cover, res.map_err(|_| ())),
                        ));
                        rows.push(Row {
                            artist,
                            cover: CoverState::Loading,
                        });
                    } else {
                        rows.push(Row {
                            artist,
                            cover: CoverState::NotLoaded,
                        });
                    }
                }
                self.rows = rows;
                self.loaded = true;
                if tasks.is_empty() {
                    Action::none()
                } else {
                    Action::task(Task::batch(tasks))
                }
            }
            Msg::ArtistsLoadFailed => {
                warn!("failed to load artists");
                self.rows = Vec::new();
                self.loaded = true;
                Action::none()
            }
            Msg::ArtistClicked(id) => {
                debug!(id = %id.as_str(), "artist clicked");
                Action::none()
            }
            Msg::CoverLoaded(id, result) => {
                let Some(row) = self
                    .rows
                    .iter_mut()
                    .find(|row| row.artist.cover.as_ref() == Some(&id))
                else {
                    return Action::none();
                };
                row.cover = match result {
                    Ok(bytes) => CoverState::Loaded(image::Handle::from_bytes(bytes)),
                    Err(_) => CoverState::Failed,
                };
                Action::none()
            }
            Msg::PlayShuffled(id) => Action::emit(Event::PlayShuffled(id)),
        }
    }

    fn view(&self) -> iced::Element<'_, Self::Msg> {
        if !self.loaded {
            return text("loading...").into();
        }
        if self.rows.is_empty() {
            return container(text("No artists found").color([0.6, 0.6, 0.6]))
                .center(Fill)
                .into();
        }
        let list = table(
            [
                table::column(text(""), |row: Row| cover_cell(row)),
                table::column(text("Artist"), |row: Row| {
                    Element::from(
                        button(text(row.artist.name.clone()))
                            .on_press(Msg::ArtistClicked(row.artist.id.clone())),
                    )
                })
                .width(Fill),
                table::column(text("Albums"), |row: Row| {
                    text(row.artist.album_count.to_string())
                }),
                table::column(text(""), |row: Row| shuffle_button(row)),
            ],
            self.rows.clone(),
        )
        .width(Fill);
        scrollable(list).width(Fill).height(Fill).into()
    }
}

fn cover_cell(row: Row) -> Element<'static, Msg> {
    match &row.cover {
        CoverState::Loaded(handle) => image(handle.clone())
            .filter_method(image::FilterMethod::Linear)
            .width(COVER_DISPLAY)
            .height(COVER_DISPLAY)
            .into(),
        CoverState::Loading => text("…").into(),
        // No art (offline libraries) or a failed fetch: neutral placeholder.
        _ => container(lucide::music().size(20).color([0.5, 0.5, 0.5]))
            .width(COVER_DISPLAY)
            .height(COVER_DISPLAY)
            .align_x(Center)
            .align_y(Center)
            .into(),
    }
}

fn shuffle_button(row: Row) -> Element<'static, Msg> {
    button(lucide::shuffle().size(16))
        .on_press(Msg::PlayShuffled(row.artist.id.clone()))
        .into()
}
