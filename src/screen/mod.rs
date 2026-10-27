//! Screen construction convention.
//!
//! Screens are plain state structs with inherent methods. There is no `Screen`
//! trait and no associated context type.
//!
//! # Shape
//!
//! Each screen module exposes:
//!
//! - `State`: local UI state only.
//! - `Msg`: messages produced by its own view or async work.
//! - `Event`: intents for the parent (`MainScreen`).
//! - `init`: constructs `State`, optionally returning a `Task<Msg>`.
//! - `update`: mutates local state and returns `Action<Msg, Event>`.
//! - `view`: builds the element.
//! - `subscription`: optional; batched by the parent screen.
//!
//! Shared read-only state is passed as a named `Ctx` when several values are
//! needed, or as direct parameters when only one or two are needed. `Ctx` is
//! `Copy`, read-only, built per call, and never stored in `State`.
//!
//! ```ignore
//! pub fn init(id: AlbumId) -> (State, Task<Msg>);
//!
//! pub fn update(
//!     state: &mut State,
//!     ctx: Ctx<'_>,
//!     msg: Msg,
//! ) -> Action<Msg, Event>;
//!
//! pub fn view<'a>(
//!     state: &'a State,
//!     ctx: Ctx<'a>,
//! ) -> Element<'a, Msg>;
//! ```
//!
//! # Rules
//!
//! - Keep only local UI state in `State`. Do not store `Arc<Manager>`, `Engine`,
//!   or any other parent-owned value in a screen.
//! - Mutations go upward as `Event`. Never pass `&mut MainScreen` to a screen.
//! - `Ctx` is read-only. If a screen needs a mutation, add an event instead of a
//!   mutable context field.
//! - `Arc::clone` is allowed only when handing a manager handle to an async
//!   task. That is a refcount bump, not a clone of UI state.
//! - Add fields to `Ctx` only when a screen actually reads them.
//! - A screen's `subscription` is `pub(in crate::screen)` so `dead_code` warns
//!   when its parent forgets to batch it. Do not add `#[deny(dead_code)]`.
//! - Register new screens in `SubScreen`, `screen::Msg`,
//!   `MainScreen::update`, `MainScreen::view_route`, and
//!   `MainScreen::make_screen`.
//!
//! # Migration note
//!
//! `change_docs/screen_01.md` documents the move away from the `Screen` trait.
//! Use the convention above for new screens; do not reintroduce a screen trait
//! just to unify their signatures.

use std::time::Duration;

use bytes::Bytes;
use iced::Subscription;
use iced::futures::{SinkExt, Stream};
use iced::widget::pane_grid;
use iced_fonts::lucide;

#[cfg(feature = "dev")]
pub mod debug;

use crate::config::{self, PanelScreenConfig, PanelStartup, UiSettings};
use crate::engine::{self, Engine, EngineEvent, EngineMsg};
use crate::library::{AlbumId, ArtistId, CoverId, Manager};
use crate::prelude::*;
use crate::widget::stack::{StackEntry, stack};

pub mod album_detail;
pub mod artist_detail;
pub mod artists;
pub mod library_setup;
pub mod now_playing_panel;
pub mod queue;
pub mod settings;
pub mod sidebar;

/// Read-only shared state passed to a child screen for one call.
///
/// Built immediately before `update`/`view` from `MainScreen`'s fields. It is
/// never stored in a screen, and it contains only references or `Copy` data.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub manager: &'a Arc<Manager>,
    pub engine: &'a Engine,
    pub now_playing: Option<&'a engine::NowPlaying>,
}

/// Engine tick period — fast enough to notice track-end within ~1/4 s and keep
/// the bar smooth.
const TICK: Duration = Duration::from_millis(250);

/// Logical cover size in the bar (mirrors `artists::COVER_DISPLAY`).
const COVER_DISPLAY: f32 = 48.0;

/// Pixel size requested from the library (2× the display size for HiDPI; the
/// offline store returns art as stored, a networked provider downsizes).
const COVER_PX: u32 = 96;

/// Minimum fraction of the window the sidebar may occupy.
const SIDEBAR_MIN_RATIO: f32 = 0.05;

/// Minimum fraction of the window the right panel may occupy when reopened.
const PANEL_MIN_RATIO: f32 = 0.08;

/// Dragging the panel split past this point is equivalent to closing the panel.
const PANEL_COLLAPSE_RATIO: f32 = 0.985;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Artists,
    ArtistDetail(ArtistId),
    AlbumDetail(AlbumId),
    Albums,
    AllTracks,
    Playlists,
    Settings,
}

/// Screen data owned by the coordinator. One variant per screen; the `State`
/// lives here (not in the `Stack` widget) and is popped together with its
/// entry, so back-navigation restores exactly what was left behind.
#[derive(Debug)]
pub enum SubScreen {
    Artists(artists::State),
    ArtistDetail(ArtistId),
    AlbumDetail(AlbumId),
    Albums,
    AllTracks,
    Playlists,
    Settings,
}

#[derive(Debug)]
pub struct ScreenEntry {
    route: Route,
    screen: SubScreen,
}

#[derive(Debug)]
pub struct MainScreen {
    manager: Arc<Manager>,
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
    // --- Navigation (region 02) -----------------------------------------
    entries: Vec<StackEntry<ScreenEntry>>,
    next_route_id: u64,
    // --- Layout (regions 01–03) -----------------------------------------
    panes: Panes,
    ui_settings: UiSettings,
    settings_dirty: bool,
    panel_open: bool,
    panel_screen: PanelScreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Sidebar,
    Content,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelScreen {
    Queue,
    NowPlaying,
}

impl From<PanelScreenConfig> for PanelScreen {
    fn from(value: PanelScreenConfig) -> Self {
        match value {
            PanelScreenConfig::Queue => PanelScreen::Queue,
            PanelScreenConfig::NowPlaying => PanelScreen::NowPlaying,
        }
    }
}

impl From<PanelScreen> for PanelScreenConfig {
    fn from(value: PanelScreen) -> Self {
        match value {
            PanelScreen::Queue => PanelScreenConfig::Queue,
            PanelScreen::NowPlaying => PanelScreenConfig::NowPlaying,
        }
    }
}

/// The pane-grid layout: sidebar | content | optional panel.
///
/// `split` ids are retained so resize events can be attributed to the ratio
/// they actually belong to.
#[derive(Debug)]
struct Panes {
    state: pane_grid::State<Region>,
    content: pane_grid::Pane,
    sidebar_split: pane_grid::Split,
    panel_split: Option<pane_grid::Split>,
    panel_pane: Option<pane_grid::Pane>,
    /// Inner ratio of the content/panel split (not normalized to the window).
    panel_inner_ratio: f32,
}

impl Panes {
    fn build(settings: &UiSettings, panel_open: bool) -> Self {
        let (mut state, sidebar) = pane_grid::State::new(Region::Sidebar);
        let (content, sidebar_split) = state
            .split(pane_grid::Axis::Vertical, sidebar, Region::Content)
            .expect("splitting the root pane yields a new pane");

        let sidebar_ratio = settings.sidebar_ratio.clamp(SIDEBAR_MIN_RATIO, 0.5);
        state.resize(sidebar_split, sidebar_ratio);

        let mut panes = Self {
            state,
            content,
            sidebar_split,
            panel_split: None,
            panel_pane: None,
            panel_inner_ratio: 0.5,
        };
        if panel_open {
            panes.open_panel(sidebar_ratio, settings.panel_ratio);
        }
        panes
    }

    fn open_panel(&mut self, sidebar_ratio: f32, panel_ratio: f32) {
        if self.panel_split.is_some() {
            return;
        }
        if let Some((panel, split)) =
            self.state
                .split(pane_grid::Axis::Vertical, self.content, Region::Panel)
        {
            let inner = panel_inner_ratio(sidebar_ratio, panel_ratio);
            self.state.resize(split, inner);
            self.panel_split = Some(split);
            self.panel_pane = Some(panel);
            self.panel_inner_ratio = inner;
        }
    }

    fn close_panel(&mut self) {
        let Some(panel) = self.panel_pane.take() else {
            return;
        };
        if self.state.close(panel).is_some() {
            self.panel_split = None;
        }
    }
}

/// Converts a desired panel fraction of the whole window into the ratio of the
/// inner content/panel split (`0.0..=1.0`).
fn panel_inner_ratio(sidebar_ratio: f32, panel_fraction: f32) -> f32 {
    let remaining = (1.0 - sidebar_ratio).max(0.01);
    (1.0 - panel_fraction / remaining).clamp(0.05, 0.95)
}

/// Inverse of [`panel_inner_ratio`].
fn panel_fraction(sidebar_ratio: f32, inner_ratio: f32) -> f32 {
    (1.0 - sidebar_ratio) * (1.0 - inner_ratio)
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
    /// A pane divider was dragged.
    PaneResized(pane_grid::ResizeEvent),
    Sidebar(sidebar::Msg),
    /// Toggle the right panel (region 03).
    TogglePanel,
    /// Switch what the right panel shows.
    SwitchPanel(PanelScreen),
    /// Pop one entry off the navigation stack (`Alt+Left`).
    Back,
    /// Keyboard events (only `Alt+Left` is acted upon).
    Keyboard(iced::keyboard::Event),
}

impl MainScreen {
    pub fn init(manager: Arc<Manager>) -> (Self, Task<Msg>) {
        let ui_settings = config::load_ui_settings();
        let panel_open = match ui_settings.panel_startup {
            PanelStartup::PersistState => ui_settings.panel_open,
            PanelStartup::StartClosed => false,
            PanelStartup::StartOpened => true,
        };
        let panel_screen = PanelScreen::from(ui_settings.panel_screen.clone());
        let panes = Panes::build(&ui_settings, panel_open);

        let (artists_state, task) = artists::init(manager.clone());

        let entries = vec![StackEntry::new(
            0,
            ScreenEntry {
                route: Route::Artists,
                screen: SubScreen::Artists(artists_state),
            },
        )];

        (
            Self {
                engine: Engine::real(manager.clone()),
                manager,
                now_playing: None,
                cover: None,
                seeking: None,
                last_volume: 1.0,
                entries,
                next_route_id: 1,
                panes,
                ui_settings,
                settings_dirty: false,
                panel_open,
                panel_screen,
            },
            task.map(Msg::ArtistsScreenMsg),
        )
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::ArtistsScreenMsg(inner) => {
                let action = {
                    let MainScreen {
                        entries,
                        manager,
                        engine,
                        now_playing,
                        ..
                    } = self;

                    let Some(entry) = entries.last_mut() else {
                        return Task::none();
                    };
                    let SubScreen::Artists(screen) = &mut entry.value.screen else {
                        return Task::none();
                    };

                    let ctx = Ctx {
                        manager: &*manager,
                        engine: &*engine,
                        now_playing: now_playing.as_ref(),
                    };

                    artists::update(screen, ctx, inner)
                };

                match action {
                    Action::None => Task::none(),
                    Action::Task(task) => task.map(Msg::ArtistsScreenMsg),
                    Action::Emit(event) => self.handle_artists_event(event),
                }
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
                self.engine
                    .update(msg)
                    .handle(Msg::Engine, |event| Task::done(Msg::EngineEvent(event)))
            }
            Msg::EngineEvent(event) => self.apply_engine_event(event),
            Msg::Tick => {
                let (events, engine_task) = self.engine.tick();
                let mut tasks: Vec<Task<Msg>> = events
                    .into_iter()
                    .map(|event| self.apply_engine_event(event))
                    .collect();
                tasks.push(engine_task.map(Msg::Engine));

                // Debounced settings write: at most once per engine tick while
                // a divider is being dragged.
                if self.settings_dirty {
                    self.save_settings();
                }

                Task::batch(tasks)
            }
            // Ignore stale results: only the current track's cover is shown,
            // so a slow response for a track we've already left must not
            // clobber a newer cover.
            Msg::CoverLoaded(id, result) => {
                let Some(np) = &self.now_playing else {
                    return Task::none();
                };
                if np.track.cover.as_ref() != Some(&id) {
                    return Task::none();
                }
                self.cover = match result {
                    Ok(bytes) => Some(image::Handle::from_bytes(bytes)),
                    Err(_) => None, // fallback (music icon) shown instead
                };
                Task::none()
            }
            Msg::SeekDragged(position) => {
                self.seeking = Some(position);
                Task::none()
            }
            Msg::SeekReleased => {
                let Some(position) = self.seeking else {
                    return Task::none();
                };
                Task::done(Msg::Engine(EngineMsg::Seek(Duration::from_secs_f32(
                    position,
                ))))
            }
            Msg::PaneResized(event) => {
                self.panes.state.resize(event.split, event.ratio);

                if event.split == self.panes.sidebar_split {
                    self.ui_settings.sidebar_ratio = event.ratio;
                    if self.panel_split().is_some() {
                        self.ui_settings.panel_ratio =
                            panel_fraction(event.ratio, self.panes.panel_inner_ratio);
                    }
                    self.settings_dirty = true;
                } else if Some(event.split) == self.panel_split() {
                    if event.ratio >= PANEL_COLLAPSE_RATIO {
                        // Dragging the panel to zero is the same as the toggle.
                        self.panel_open = false;
                        self.ui_settings.panel_open = false;
                        self.panes.close_panel();
                        self.save_settings();
                    } else {
                        self.panes.panel_inner_ratio = event.ratio;
                        self.ui_settings.panel_ratio =
                            panel_fraction(self.ui_settings.sidebar_ratio, event.ratio);
                        self.settings_dirty = true;
                    }
                }
                Task::none()
            }
            Msg::Sidebar(sidebar_msg) => self.handle_sidebar(sidebar_msg),
            Msg::TogglePanel => {
                self.panel_open = !self.panel_open;
                self.ui_settings.panel_open = self.panel_open;
                if self.panel_open {
                    self.panes.open_panel(
                        self.ui_settings.sidebar_ratio,
                        self.ui_settings.panel_ratio.max(PANEL_MIN_RATIO),
                    );
                } else {
                    self.panes.close_panel();
                }
                self.save_settings();
                Task::none()
            }
            Msg::SwitchPanel(screen) => {
                self.panel_screen = screen;
                self.ui_settings.panel_screen = PanelScreenConfig::from(screen);
                self.save_settings();
                Task::none()
            }
            Msg::Back => {
                // The root entry is never popped: back from a root is a no-op.
                if self.entries.len() > 1 {
                    self.entries.pop();
                }
                Task::none()
            }
            Msg::Keyboard(event) => {
                let iced::keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
                    return Task::none();
                };
                let back = modifiers.alt()
                    && key == iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft);
                if back && self.entries.len() > 1 {
                    self.entries.pop();
                }
                Task::none()
            }
        }
    }

    fn handle_artists_event(&mut self, event: artists::Event) -> Task<Msg> {
        match event {
            artists::Event::PlayShuffled(id) => {
                let manager = self.manager.clone();
                Task::perform(collect_shuffled_songs(manager, id), |res| match res {
                    Ok(tracks) => Msg::Engine(EngineMsg::PlayQueue(tracks)),
                    Err(err) => Msg::EngineEvent(EngineEvent::Error(err.to_string())),
                })
            }
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let content = pane_grid(&self.panes.state, |_, pane_state, _is_maximized| {
            let content: Element<'_, Msg> = match pane_state {
                Region::Sidebar => sidebar::view(Msg::Sidebar),
                Region::Content => self.view_stack(),
                Region::Panel => self.view_panel(),
            };
            pane_grid::Content::new(content)
        })
        .on_resize(6.0, Msg::PaneResized)
        .width(Fill)
        .height(Fill);

        widget::column![content, self.view_now_playing()]
            .width(Fill)
            .height(Fill)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        Subscription::batch([
            Subscription::run(tick_stream),
            iced::keyboard::listen().map(Msg::Keyboard),
        ])
    }
}

impl MainScreen {
    fn panel_split(&self) -> Option<pane_grid::Split> {
        self.panes.panel_split
    }

    fn save_settings(&mut self) {
        if let Err(err) = config::save_ui_settings(&self.ui_settings) {
            warn!(%err, "failed to save settings.toml");
        }
        self.settings_dirty = false;
    }

    fn alloc_route_id(&mut self) -> u64 {
        let id = self.next_route_id;
        self.next_route_id += 1;
        id
    }

    fn make_screen(&self, route: &Route) -> (SubScreen, Task<Msg>) {
        match route {
            Route::Artists => {
                let (state, task) = artists::init(self.manager.clone());
                (SubScreen::Artists(state), task.map(Msg::ArtistsScreenMsg))
            }
            Route::ArtistDetail(id) => (SubScreen::ArtistDetail(id.clone()), Task::none()),
            Route::AlbumDetail(id) => (SubScreen::AlbumDetail(id.clone()), Task::none()),
            Route::Albums => (SubScreen::Albums, Task::none()),
            Route::AllTracks => (SubScreen::AllTracks, Task::none()),
            Route::Playlists => (SubScreen::Playlists, Task::none()),
            Route::Settings => (SubScreen::Settings, Task::none()),
        }
    }

    fn push_route(&mut self, route: Route) -> Task<Msg> {
        let (screen, task) = self.make_screen(&route);
        let id = self.alloc_route_id();
        self.entries
            .push(StackEntry::new(id, ScreenEntry { route, screen }));
        task
    }

    /// Main-section click: clear the stack and show that section's root.
    /// Re-clicking the current section's button truncates back to its root,
    /// keeping the surviving entry's state.
    fn goto_root(&mut self, route: Route) -> Task<Msg> {
        let is_root = self
            .entries
            .first()
            .is_some_and(|entry| entry.value.route == route);

        if is_root {
            self.entries.truncate(1);
            return Task::none();
        }

        self.entries.clear();
        self.push_route(route)
    }

    fn handle_sidebar(&mut self, msg: sidebar::Msg) -> Task<Msg> {
        match msg {
            sidebar::Msg::Artists => self.goto_root(Route::Artists),
            sidebar::Msg::Albums => self.goto_root(Route::Albums),
            sidebar::Msg::Tracks => self.goto_root(Route::AllTracks),
            sidebar::Msg::Playlists => self.goto_root(Route::Playlists),
            sidebar::Msg::Settings => {
                // Settings never clears the stack; it is pushed on top of the
                // current route. Re-clicking resets the existing settings
                // screen instead of nesting another copy.
                if let Some(entry) = self
                    .entries
                    .iter_mut()
                    .find(|entry| entry.value.route == Route::Settings)
                {
                    entry.value.screen = SubScreen::Settings;
                    Task::none()
                } else {
                    self.push_route(Route::Settings)
                }
            }
        }
    }

    fn view_stack<'a>(&'a self) -> Element<'a, Msg> {
        stack(&self.entries, |entry: &'a ScreenEntry| {
            self.view_route(entry)
        })
        .into()
    }

    fn view_route<'a>(&'a self, entry: &'a ScreenEntry) -> Element<'a, Msg> {
        match &entry.screen {
            SubScreen::Artists(screen) => artists::view(screen).map(Msg::ArtistsScreenMsg),
            SubScreen::ArtistDetail(id) => placeholder(format!("Artist {}", id.as_str())),
            SubScreen::AlbumDetail(id) => placeholder(format!("Album {}", id.as_str())),
            SubScreen::Albums => placeholder("Albums"),
            SubScreen::AllTracks => placeholder("Tracks"),
            SubScreen::Playlists => placeholder("Playlists"),
            SubScreen::Settings => placeholder("Settings"),
        }
    }

    fn view_panel(&self) -> Element<'_, Msg> {
        if !self.panel_open {
            return widget::container(widget::column![]).into();
        }

        let tabs = widget::row![
            button(text("Queue"))
                .on_press(Msg::SwitchPanel(PanelScreen::Queue))
                .style(panel_tab_style(self.panel_screen == PanelScreen::Queue)),
            button(text("Now playing"))
                .on_press(Msg::SwitchPanel(PanelScreen::NowPlaying))
                .style(panel_tab_style(
                    self.panel_screen == PanelScreen::NowPlaying
                )),
        ]
        .spacing(4);

        let body: Element<'_, Msg> = match self.panel_screen {
            PanelScreen::Queue => placeholder("Queue"),
            PanelScreen::NowPlaying => placeholder("Now playing"),
        };

        widget::column![
            widget::row![
                tabs,
                widget::space::horizontal(),
                button(text("Hide")).on_press(Msg::TogglePanel)
            ]
            .spacing(4),
            body,
        ]
        .spacing(8)
        .padding(8)
        .height(Fill)
        .into()
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

        // --- Right: mute toggle + volume + panel toggle ----------------------
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
        let right = row![
            mute_button,
            volume_slider,
            button(text("Panel")).on_press(Msg::TogglePanel)
        ]
        .align_y(Center)
        .spacing(8);

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
    fn apply_engine_event(&mut self, event: EngineEvent) -> Task<Msg> {
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
        task
    }
}

fn placeholder(title: impl Into<String>) -> Element<'static, Msg> {
    container(text(title.into()).size(16))
        .padding(16)
        .width(Fill)
        .height(Fill)
        .into()
}

fn panel_tab_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, _status| {
        let palette = theme.extended_palette();
        button::Style {
            background: if active {
                Some(palette.primary.weak.color.into())
            } else {
                None
            },
            text_color: if active {
                palette.primary.weak.text
            } else {
                palette.background.base.text
            },
            ..button::Style::default()
        }
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
