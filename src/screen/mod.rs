use std::time::Duration;

use bytes::Bytes;
use iced::Subscription;
use iced::futures::{SinkExt, Stream};
use iced_fonts::lucide;

use crate::engine::{self, Engine, EngineEvent, EngineMsg};
use crate::library::{ArtistId, CoverId, Manager};
use crate::prelude::*;

pub mod artists;
pub mod library_setup;

/// Engine tick period — fast enough to notice track-end within ~1/4 s and keep
/// the bar smooth.
const TICK: Duration = Duration::from_millis(250);

/// Logical cover size in the bar (mirrors `artists::COVER_DISPLAY`).
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2× the display size for HiDPI; the
/// offline store returns art as stored, a networked provider downsizes).
const COVER_PX: u32 = 96;

#[derive(Debug)]
pub struct MainScreen {
    manager: Arc<Manager>,
    current_screen: SubScreen,
    engine: Engine,
    /// Snapshot pushed by the engine; the bar renders from this.
    now_playing: Option<engine::NowPlaying>,
    /// Current track's cover art. The handle is built **once** when the bytes
    /// arrive and reused every frame (see the comment in `artists.rs`).
    cover: Option<image::Handle>,
    /// Draft seek position (seconds) while the seek slider is being dragged,
    /// so the 250 ms tick doesn't yank the thumb.
    seeking: Option<f32>,
    /// Volume restored by the mute toggle, default `1.0`.
    last_volume: f32,
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
    /// Cover fetch result for a cover id (mirrors `artists::Msg::CoverLoaded`).
    CoverLoaded(CoverId, Result<Bytes, ()>),
    /// The seek slider moved while dragging — stores the draft position.
    SeekDragged(f32),
    /// The seek slider was released — commits the draft seek to the engine.
    SeekReleased,
}

impl Screen for MainScreen {
    type Msg = Msg;

    type Emit = Never;

    type InitData = Arc<Manager>;

    fn init(data: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        let manager = data;
        let (state, action) = artists::State::init(manager.clone());
        let task = action.handle(Msg::ArtistsScreenMsg, |_event| Task::none());
        (
            Self {
                engine: Engine::real(manager.clone()),
                manager,
                current_screen: SubScreen::Artists(state),
                now_playing: None,
                cover: None,
                seeking: None,
                last_volume: 1.0,
            },
            Action::task(task),
        )
    }

    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit> {
        match msg {
            Msg::ArtistsScreenMsg(inner) => {
                let SubScreen::Artists(screen) = &mut self.current_screen;
                let manager = self.manager.clone();
                let task = screen
                    .update(inner)
                    .handle(Msg::ArtistsScreenMsg, move |event| match event {
                        artists::Event::PlayShuffled(id) => {
                            let manager = manager.clone();
                            Task::perform(collect_shuffled_songs(manager, id), |res| match res {
                                Ok(tracks) => Msg::Engine(EngineMsg::PlayQueue(tracks)),
                                Err(err) => Msg::EngineEvent(EngineEvent::Error(err.to_string())),
                            })
                        }
                    });
                Action::task(task)
            }
            Msg::Engine(msg) => {
                // Mute-toggle bookkeeping: remember the pre-mute volume when
                // silence is requested from a non-zero level, so the toggle
                // can restore it.
                if let Some(np) = &self.now_playing
                    && matches!(&msg, EngineMsg::SetVolume(volume) if *volume == 0.0)
                    && np.volume > 0.0
                {
                    self.last_volume = np.volume;
                }
                Action::task(
                    self.engine
                        .update(msg)
                        .handle(Msg::Engine, |event| Task::done(Msg::EngineEvent(event))),
                )
            }
            Msg::EngineEvent(event) => self.apply_engine_event(event),
            Msg::Tick => {
                let (events, engine_task) = self.engine.tick();
                let mut tasks: Vec<Task<Msg>> = events
                    .into_iter()
                    .map(|event| self.apply_engine_event(event))
                    .map(|action| action.handle(|msg| msg, |_| Task::none()))
                    .collect();
                tasks.push(engine_task.map(Msg::Engine));
                Action::task(Task::batch(tasks))
            }
            // Ignore stale results: only the current track's cover is shown,
            // so a slow response for a track we've already left must not
            // clobber a newer cover.
            Msg::CoverLoaded(id, result) => {
                let Some(np) = &self.now_playing else {
                    return Action::none();
                };
                if np.track.cover.as_ref() != Some(&id) {
                    return Action::none();
                }
                self.cover = match result {
                    Ok(bytes) => Some(image::Handle::from_bytes(bytes)),
                    Err(_) => None, // fallback (music icon) shown instead
                };
                Action::none()
            }
            Msg::SeekDragged(position) => {
                self.seeking = Some(position);
                Action::none()
            }
            Msg::SeekReleased => {
                let Some(position) = self.seeking else {
                    return Action::none();
                };
                Action::task(Task::done(Msg::Engine(EngineMsg::Seek(
                    Duration::from_secs_f32(position),
                ))))
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
        let np = &self.now_playing.as_ref();

        // --- Left: cover + title/artist ------------------------------------
        let cover: Element<'_, Msg> = match &self.cover {
            Some(handle) => image(handle.clone())
                .filter_method(image::FilterMethod::Linear)
                .width(COVER_DISPLAY)
                .height(COVER_DISPLAY)
                .into(),
            // Fallback while loading / when the track has no art.
            None => container(lucide::music().size(24).color([0.5, 0.5, 0.5]))
                .width(COVER_DISPLAY)
                .height(COVER_DISPLAY)
                .align_x(Center)
                .align_y(Center)
                .style(muted_cover_style)
                .into(),
        };
        let info = widget::column![
            text(np.map(|t| t.track.title.clone()).unwrap_or_default()),
            text(
                np.map(|t| t.track.artist.clone().unwrap_or_default())
                    .unwrap_or_default()
            )
            .size(12)
            .color([0.6, 0.6, 0.6]),
        ]
        .spacing(2);
        let left = row![cover, info].align_y(Center).spacing(8);

        // --- Center: transport + progress -----------------------------------
        let play_pause = match np.map(|n| n.state) {
            Some(engine::PlaybackState::Paused) => {
                button(lucide::play().size(16)).on_press(Msg::Engine(EngineMsg::Resume))
            }
            Some(_) => button(lucide::pause().size(16)).on_press(Msg::Engine(EngineMsg::Pause)),
            None => button(lucide::play().size(16)),
        };
        let transport = row![
            button(lucide::skip_back().size(16))
                .on_press_maybe(np.map(|_| Msg::Engine(EngineMsg::Prev))),
            play_pause,
            button(lucide::skip_forward().size(16))
                .on_press_maybe(np.map(|_| Msg::Engine(EngineMsg::Next))),
            button(lucide::square().size(16))
                .on_press_maybe(np.map(|_| Msg::Engine(EngineMsg::Stop))),
        ]
        .spacing(8)
        .align_y(Center);

        // Guard degenerate ranges and clamp the position into it.
        let total_secs = np
            .map(|n| n.duration.as_secs_f32().max(1.0))
            .unwrap_or_default();
        let position_secs = np
            .map(|n| n.position.as_secs_f32().min(total_secs))
            .unwrap_or_default();
        // While dragging, show the draft instead of the ticking snapshot.
        let slider_value = self.seeking.unwrap_or(position_secs);
        let duration = np.map(|n| n.duration).unwrap_or_default();
        let position = np.map(|n| n.position).unwrap_or_default();
        let progress = row![
            text(fmt_duration(position)),
            slider(0.0..=total_secs, slider_value, Msg::SeekDragged)
                .on_release(Msg::SeekReleased)
                .width(300),
            text(fmt_duration(duration)),
        ]
        .spacing(8)
        .align_y(Center);

        let center = widget::column![transport, progress]
            .width(Fill)
            .align_x(Center)
            .spacing(8);

        // --- Right: mute toggle + volume -------------------------------------
        let volume = np.map(|n| n.volume).unwrap_or(1.0);
        let muted = volume <= 0.0;
        let mute_button = button(if muted {
            lucide::volume_x().size(16)
        } else {
            lucide::volume_two().size(16)
        })
        .on_press(Msg::Engine(EngineMsg::SetVolume(if muted {
            self.last_volume
        } else {
            0.0
        })));
        let volume_slider = slider(0.0..=1.0, volume, |v| Msg::Engine(EngineMsg::SetVolume(v)))
            .step(0.01_f32)
            .width(100);
        let right = row![mute_button, volume_slider].align_y(Center).spacing(8);

        row![left, center, right]
            .align_y(Center)
            .spacing(12)
            .padding(8)
            .width(Fill)
            .into()
    }

    /// Log an engine event and refresh the now-playing snapshot pushed to the
    /// bar. Returns any task the event spawns (e.g. a cover fetch — this is
    /// the "watch the events flow" verification of Facet 1).
    fn apply_engine_event(&mut self, event: EngineEvent) -> Action<Msg, Never> {
        debug!(?event, "engine event");
        let mut task = Task::none();
        match event {
            EngineEvent::PlaybackStarted(track) => {
                self.cover = None;
                self.seeking = None;
                task = if let Some(cover) = track.cover.clone() {
                    let manager = self.manager.clone();
                    let fetch_cover = cover.clone();
                    Task::perform(
                        async move { manager.cover(&fetch_cover, COVER_PX).await },
                        move |res| Msg::CoverLoaded(cover, res.map_err(|_| ())),
                    )
                } else {
                    Task::none()
                };
            }
            EngineEvent::Stopped => {
                self.cover = None;
                self.seeking = None;
            }
            EngineEvent::SeekDone => {
                self.seeking = None;
            }
            EngineEvent::TrackFinished(_child) => {}
            EngineEvent::Paused => {}
            EngineEvent::Resumed => {}
            EngineEvent::UpdateNowPlaying => {}
            EngineEvent::QueueChanged => {}
            EngineEvent::Error(_e) => {}
        }

        self.now_playing = self.engine.now_playing();
        Action::task(task)
    }
}

/// Muted fill for the placeholder cover slot (while loading / no art).
fn muted_cover_style(theme: &iced::Theme) -> widget::container::Style {
    widget::container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        ..Default::default()
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

/// Fetch an artist's albums, collect the songs, shuffle.
///
/// Shuffle lives in the caller for now, not the engine. A neutral
/// [`crate::library::ArtistDetail`] already lists albums — no `is_dir` filter
/// (splits live inside a single library, not across id hierarchies).
async fn collect_shuffled_songs(
    manager: Arc<Manager>,
    artist_id: ArtistId,
) -> Result<Vec<engine::Track>, engine::EngineError> {
    let detail = manager.artist(&artist_id).await?;
    let mut songs = Vec::new();
    for album in detail.albums {
        songs.extend(manager.album(&album.id).await?.songs);
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
