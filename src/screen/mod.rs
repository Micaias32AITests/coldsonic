use std::time::Duration;

use async_compat::Compat;
use iced::Subscription;
use iced::futures::{SinkExt, Stream};
use iced_fonts::lucide;
use opensubsonic::Client;

use crate::engine::{self, Engine, EngineEvent, EngineMsg};
use crate::prelude::*;

pub mod artists;
pub mod login;

/// Engine tick period — fast enough to notice track-end within ~1/4 s and keep
/// the bar smooth.
const TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct MainScreen {
    client: Arc<Client>,
    current_screen: SubScreen,
    engine: Engine,
    /// Snapshot pushed by the engine; the bar renders from this.
    now_playing: Option<engine::NowPlaying>,
}

#[derive(Debug, Clone)]
pub enum SubScreen {
    Artists(artists::State),
}

#[derive(Debug, Clone)]
pub enum Msg {
    ArtistsScreenMsg(artists::Msg),
    /// UI intents routed into the engine.
    Engine(engine::EngineMsg),
    /// Engine events (from `Engine::update`) applied to the UI state.
    EngineEvent(engine::EngineEvent),
    /// Produced by the 250 ms subscription; drives `engine.tick()`.
    Tick,
}

impl Screen for MainScreen {
    type Msg = Msg;

    type Emit = Never;

    type InitData = Client;

    fn init(data: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        let client: Arc<_> = data.into();
        let (state, action) = artists::State::init(client.clone());
        let task = action.handle(Msg::ArtistsScreenMsg, |_event| Task::none());
        (
            Self {
                client,
                current_screen: SubScreen::Artists(state),
                engine: Engine::mock(),
                now_playing: None,
            },
            Action::task(task),
        )
    }

    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit> {
        match msg {
            Msg::ArtistsScreenMsg(inner) => {
                let SubScreen::Artists(screen) = &mut self.current_screen;
                let client = self.client.clone();
                let task = screen
                    .update(inner)
                    .handle(Msg::ArtistsScreenMsg, move |event| match event {
                        artists::Event::PlayShuffled(id) => {
                            let client = client.clone();
                            Task::perform(Compat::new(collect_shuffled_songs(client, id)), |res| {
                                match res {
                                    Ok(tracks) => Msg::Engine(EngineMsg::PlayQueue(tracks)),
                                    Err(err) => {
                                        Msg::EngineEvent(EngineEvent::Error(err.to_string()))
                                    }
                                }
                            })
                        }
                    });
                Action::task(task)
            }
            Msg::Engine(msg) => Action::task(
                self.engine
                    .update(msg)
                    .handle(Msg::Engine, |event| Task::done(Msg::EngineEvent(event))),
            ),
            Msg::EngineEvent(event) => {
                self.apply_engine_event(event);
                Action::none()
            }
            Msg::Tick => {
                for event in self.engine.tick() {
                    self.apply_engine_event(event);
                }
                Action::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Msg> {
        widget::column![self.view_subscreen(), self.view_now_playing()]
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Msg> {
        Subscription::run(tick_stream)
    }
}

impl MainScreen {
    fn view_subscreen(&self) -> Element<'_, Msg> {
        match &self.current_screen {
            SubScreen::Artists(screen) => screen.view().map(Msg::ArtistsScreenMsg),
        }
    }

    fn view_now_playing(&self) -> Element<'_, Msg> {
        match &self.now_playing {
            Some(np) => row![
                // Facet 1: the mock owns no cover bytes → placeholder space.
                space().width(48.0).height(48.0),
                widget::column![
                    text(np.track.title.clone()),
                    text(np.track.artist.clone().unwrap_or_default())
                        .size(12)
                        .color([0.6, 0.6, 0.6]),
                ]
                .spacing(2),
                text(format!(
                    "{} / {}",
                    fmt_duration(np.position),
                    fmt_duration(np.duration)
                )),
                match np.state {
                    engine::PlaybackState::Paused => {
                        button(lucide::play().size(16)).on_press(Msg::Engine(EngineMsg::Resume))
                    }
                    _ => {
                        button(lucide::pause().size(16)).on_press(Msg::Engine(EngineMsg::Pause))
                    }
                },
                button(lucide::skip_forward().size(16)).on_press(Msg::Engine(EngineMsg::Next)),
            ]
            .align_y(Center)
            .spacing(8)
            .padding(8)
            .width(Fill)
            .into(),
            None => text("Nothing playing").into(),
        }
    }

    /// Log an engine event and refresh the now-playing snapshot pushed to the
    /// bar. This is the "watch the events flow" verification of Facet 1.
    fn apply_engine_event(&mut self, event: EngineEvent) {
        eprintln!("[engine] event: {event:?}");
        self.now_playing = self.engine.now_playing();
    }
}

/// Emits `Msg::Tick` every `TICK` as long as the app is alive. No engine data
/// crosses into the subscription; `update` reads the engine.
fn tick_stream() -> impl Stream<Item = Msg> {
    iced::stream::channel(
        1,
        |mut output: iced::futures::channel::mpsc::Sender<Msg>| async move {
            loop {
                smol::Timer::after(TICK).await;
                if output.send(Msg::Tick).await.is_err() {
                    break; // the app has shut down
                }
            }
        },
    )
}

/// Fetch an artist's albums, collect the playable songs (`!is_dir`), shuffle.
///
/// Shuffle lives in the caller for now, not the engine.
async fn collect_shuffled_songs(
    client: Arc<Client>,
    artist_id: String,
) -> Result<Vec<engine::Track>, engine::EngineError> {
    let artist = client.get_artist(&artist_id).await?;
    let mut songs = Vec::new();
    for album in artist.album {
        let album_songs = client.get_album(&album.id).await?;
        songs.extend(album_songs.song.into_iter().filter(|song| !song.is_dir));
    }
    fisher_yates(&mut songs, &mut rand::rng()); // sure bro
    Ok(songs)
}

/// Fisher–Yates shuffle.
// hope i understand this stuff one day. my brain is not braining right now
fn fisher_yates<T>(slice: &mut [T], rng: &mut impl rand::Rng) {
    for i in (1..slice.len()).rev() {
        let j = rng.random_range(0..=i);
        slice.swap(i, j);
    }
}

fn fmt_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}
