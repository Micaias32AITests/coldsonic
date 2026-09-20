use async_compat::Compat;
use bytes::Bytes;
use iced_fonts::lucide;
use opensubsonic::Client;
use opensubsonic::data::ArtistId3;

use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct State {
    client: Arc<Client>,
    rows: Vec<Row>,
}

#[derive(Debug, Clone)]
struct Row {
    artist: ArtistId3,
    cover: CoverState,
}

/// Logical size of the cover cell in the table.
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the server (2x the display size for HiDPI).
///
/// The server returns a downsized image, which keeps the download, decode and
/// GPU upload small.
const COVER_PX: i32 = 96;

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
    ArtistsLoaded(Vec<ArtistId3>),
    ArtistsLoadFailed,
    ArtistClicked(String),
    CoverLoaded(String, Result<Bytes, ()>),
    PlayShuffled(String),
}

impl Screen for State {
    type Msg = Msg;

    type Emit = Never;

    type InitData = Arc<Client>;

    fn init(client: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        let client_for_load = client.clone();
        (
            Self {
                rows: Vec::new(),
                client,
            },
            Action::task(Task::perform(
                Compat::new(async move { client_for_load.get_artists(None).await }),
                |res| match res {
                    Ok(artists) => Msg::ArtistsLoaded(
                        artists
                            .index
                            .into_iter()
                            .flat_map(|index| index.artist)
                            .collect(),
                    ),
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
                eprintln!("loading artists");
                for artist in artists.into_iter() {
                    if artist.cover_art.is_some() {
                        let client = self.client.clone();
                        let id = artist.id.clone();
                        let fetch_id = id.clone();
                        tasks.push(Task::perform(
                            Compat::new(async move {
                                client.get_cover_art(&fetch_id, Some(COVER_PX)).await
                            }),
                            move |res| Msg::CoverLoaded(id, res.map_err(|_| ())),
                        ));
                    }
                    rows.push(Row {
                        cover: if artist.cover_art.is_some() {
                            CoverState::Loading
                        } else {
                            CoverState::NotLoaded
                        },
                        artist,
                    });
                }
                self.rows = rows;
                if tasks.is_empty() {
                    Action::none()
                } else {
                    Action::task(Task::batch(tasks))
                }
            }
            Msg::ArtistsLoadFailed => {
                eprintln!("[artists] failed to load artists");
                Action::none()
            }
            Msg::ArtistClicked(id) => {
                println!("[artists] artist clicked: {id}");
                Action::none()
            }
            Msg::CoverLoaded(id, result) => {
                eprintln!("loaded cover for {id}");
                let Some(row) = self.rows.iter_mut().find(|row| row.artist.id == id) else {
                    return Action::none();
                };
                row.cover = match result {
                    Ok(bytes) => CoverState::Loaded(image::Handle::from_bytes(bytes)),
                    Err(_) => {
                        eprintln!("failed to load {id}");
                        CoverState::Failed
                    }
                };
                Action::none()
            }
            Msg::PlayShuffled(id) => {
                println!("[artists] play shuffled: {id}");
                Action::none()
            }
        }
    }

    fn view(&self) -> iced::Element<'_, Self::Msg> {
        if self.rows.is_empty() {
            return text("loading...").into();
        }
        let list = table(
            [
                table::column(text(""), |row: Row| cover_cell(row)),
                table::column(text("Artist"), |row: Row| {
                    Element::from(
                        button(text(row.artist.name))
                            .on_press(Msg::ArtistClicked(row.artist.id.clone())),
                    )
                })
                .width(Fill),
                table::column(text("Albums"), |row: Row| text(album_count(&row.artist))),
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
        _ => text("loading").into(),
    }
}

fn album_count(artist: &ArtistId3) -> String {
    artist
        .album_count
        .map(|n| n.to_string())
        .unwrap_or_else(|| "–".to_string())
}

fn shuffle_button(row: Row) -> Element<'static, Msg> {
    button(lucide::shuffle().size(16))
        .on_press(Msg::PlayShuffled(row.artist.id.clone()))
        .into()
}
