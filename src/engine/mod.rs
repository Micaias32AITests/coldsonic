//! Player engine: the seam between the UI and playback.
//!
//! The engine is a GPUI entity. Screens take its handle from the
//! [`EngineHandle`] global, call command methods (which take `&mut Context`)
//! and either subscribe to [`EngineEvent`]s or read [`Engine::now_playing`].
//! They never touch audio.
//!
//! [`EngineMsg`] is internal plumbing between the entity and the playback
//! core (`mock`/`real`); the entity is the only thing that turns a response
//! into an emitted event or a byte fetch.

pub mod mock;
pub mod player;
pub mod queue;
pub mod real;

use std::time::Duration;

use crate::engine::player::{TrackSamples, decode_to_samples};
use crate::library::{ManagerHandle, TrackId};
use crate::runtime::RT;

use gpui_kit::{App, Context, Entity, EventEmitter, Global};
pub use queue::{Queue, QueueEntry, QueueEntryId};
use rand::{Rng, RngExt};

/// How often the entity ticks playback. Matches the old UI subscription.
const TICK_INTERVAL: Duration = Duration::from_millis(250);

/// What we play — the neutral track every provider hands over.
pub type Track = crate::library::Track;

/// Internal commands between the entity and the playback core.
#[derive(Debug, Clone)]
pub enum EngineMsg {
    /// Replace the queue, start at a specific track.
    PlayQueue(Vec<Track>, usize),
    /// Play a track immediately. The current track moves to history; the
    /// upcoming queue is not modified.
    PlayNow(Track),
    /// Append to the queue end.
    Enqueue(Track),
    /// Skip to the next queued track.
    Next,
    /// Go back: restart the current track when it's past the threshold,
    /// otherwise step back into the playback history.
    ///
    /// Engine-level command: the queue exposes `step_back()` and the restart
    /// path separately, because the choice needs the playback position.
    Prev,
    Pause,
    Resume,
    Toggle,
    Stop,
    SetVolume(f32),
    /// Best-effort.
    Seek(Duration),
    /// Internal: async fetch finished.
    TrackReady {
        generation: u64,
        result: Result<TrackSamples, EngineError>,
    },
}

/// Events the engine pushes up to its owner. Screens subscribe to the entity
/// through [`EventEmitter`] and `Context::subscribe`.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    PlaybackStarted(Track),
    /// Scrobble-submission hook (Facet 4).
    TrackFinished(Track),
    Paused,
    Resumed,
    Stopped,
    SeekDone,
    UpdateNowPlaying,
    Error(String),
}

/// What the playback core asks the entity to do after an update or tick.
///
/// The core never touches a runtime: async work leaves as a request and comes
/// back as an [`EngineMsg`], so the entity decides how to run it.
#[derive(Debug, Clone, Default)]
pub enum EngineResponse {
    /// Nothing to do.
    #[default]
    Nothing,
    /// An event to push to the UI.
    Event(EngineEvent),
    /// The core needs the bytes of `id` before playback can start. The entity
    /// fetches them and feeds `EngineMsg::BytesReady { generation, .. }` back.
    LoadTrack { generation: u64, id: TrackId },
}

#[atomic_enum::atomic_enum]
#[derive(PartialEq)]
pub enum Monoization {
    None = 0,
    Mix,
    OnlyLeft,
    OnlyRight,
}

#[atomic_enum::atomic_enum]
#[derive(PartialEq)]
pub enum RepeatMode {
    None = 0,
    All,
    One,
}

impl From<EngineEvent> for EngineResponse {
    fn from(event: EngineEvent) -> Self {
        Self::Event(event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Loading,
    Playing,
    Paused,
}

/// Snapshot pushed to the UI each tick.
#[derive(Debug, Clone)]
pub struct NowPlaying {
    pub track: Track,
    pub position: Duration,
    pub duration: Duration,
    pub state: PlaybackState,
}

/// Errors surfaced by the engine. Clone so [`EngineMsg::BytesReady`] can be
/// handed back by the owner's fetch task (`std::io::Error` is not Clone).
#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    Library(String),
    #[error("decode: {0}")]
    Decode(String),
}

impl From<crate::library::LibraryError> for EngineError {
    fn from(err: crate::library::LibraryError) -> Self {
        Self::Library(err.to_string())
    }
}

/// The engine entity. Not `Clone`: the real variant owns a `!Send` device
/// sink, which is why the engine must live on the UI thread as an entity.
#[derive(Debug)]
pub enum Engine {
    Real(real::RealEngine),
}

/// Global handle to the one engine entity. Views clone the `Entity` out of it;
/// the global keeps the engine alive for the app's lifetime.
#[derive(Clone)]
pub struct EngineHandle(pub Entity<Engine>);

impl Global for EngineHandle {}

impl EngineHandle {
    pub fn get(cx: &App) -> Entity<Engine> {
        cx.global::<EngineHandle>().0.clone()
    }
}

impl EventEmitter<EngineEvent> for Engine {}

impl Engine {
    /// Build the engine and start its tick loop. The loop holds a weak handle,
    /// so it stops when the entity is dropped.
    pub fn real(cx: &mut Context<Self>, manager: ManagerHandle) -> Self {
        Self::start_tick_loop(cx);
        Self::Real(real::RealEngine::open(manager))
    }

    // ---- commands: the public API views call -------------------------------

    pub fn play_queue(&mut self, cx: &mut Context<Self>, queue: Vec<Track>, start_pos: usize) {
        self.dispatch(EngineMsg::PlayQueue(queue, start_pos), cx);
    }

    pub fn play_queue_shuffled(&mut self, cx: &mut Context<Self>, mut queue: Vec<Track>) {
        fisher_yates(&mut queue, &mut rand::rng());
        self.dispatch(EngineMsg::PlayQueue(queue, 0), cx);
    }

    pub fn play_now(&mut self, cx: &mut Context<Self>, track: Track) {
        self.dispatch(EngineMsg::PlayNow(track), cx);
    }

    pub fn enqueue(&mut self, cx: &mut Context<Self>, track: Track) {
        self.dispatch(EngineMsg::Enqueue(track), cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.dispatch(EngineMsg::Next, cx);
    }

    pub fn prev(&mut self, cx: &mut Context<Self>) {
        self.dispatch(EngineMsg::Prev, cx);
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.dispatch(EngineMsg::Pause, cx);
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        self.dispatch(EngineMsg::Resume, cx);
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.dispatch(EngineMsg::Toggle, cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.dispatch(EngineMsg::Stop, cx);
    }

    pub fn volume(&self) -> f32 {
        match self {
            Engine::Real(real) => real.volume(),
        }
    }

    pub fn set_volume(&mut self, cx: &mut Context<Self>, volume: f32) {
        self.dispatch(EngineMsg::SetVolume(volume), cx);
    }

    pub fn repeat(&self) -> RepeatMode {
        match self {
            Engine::Real(real) => real.repeat(),
        }
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        match self {
            Engine::Real(real) => real.set_repeat(repeat),
        }
    }

    pub fn seek(&mut self, cx: &mut Context<Self>, duration: Duration) {
        self.dispatch(EngineMsg::Seek(duration), cx);
    }

    // ---- read-only state ---------------------------------------------------

    pub fn now_playing(&self) -> Option<NowPlaying> {
        match self {
            Engine::Real(engine) => engine.now_playing(),
        }
    }

    /// Borrowed iteration over the upcoming queue, in playback order.
    ///
    /// There is deliberately no mutable accessor: mutations go through the
    /// command methods.
    pub fn queue(&self) -> std::collections::vec_deque::Iter<'_, QueueEntry> {
        match self {
            Engine::Real(engine) => engine.queue(),
        }
    }

    // ---- entity plumbing ---------------------------------------------------

    /// Drive playback; called by the tick loop, not by views.
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        let (events, response) = match self {
            Engine::Real(engine) => engine.tick(),
        };
        for event in events {
            cx.emit(event);
        }
        self.respond(response, cx);
    }

    fn dispatch(&mut self, msg: EngineMsg, cx: &mut Context<Self>) {
        let response = match self {
            Engine::Real(engine) => engine.update(msg),
        };
        self.respond(response, cx);
    }

    /// Turn a core response into an emitted event or a byte fetch.
    fn respond(&mut self, response: EngineResponse, cx: &mut Context<Self>) {
        match response {
            EngineResponse::Nothing => {}
            EngineResponse::Event(event) => cx.emit(event),
            EngineResponse::LoadTrack { generation, id } => {
                self.fetch_track(generation, id, cx);
            }
        }
    }

    /// The fetch task's completion. Only the matching generation is applied by
    /// the core, so a superseded load is safe.
    fn bytes_ready(
        &mut self,
        cx: &mut Context<Self>,
        generation: u64,
        result: Result<TrackSamples, EngineError>,
    ) {
        self.dispatch(EngineMsg::TrackReady { generation, result }, cx);
    }

    fn fetch_track(&self, generation: u64, id: TrackId, cx: &mut Context<Self>) {
        let manager = match self {
            Engine::Real(engine) => engine.manager().clone(),
        };
        let rt = RT::get(cx);
        let (tx, rx) = tokio::sync::oneshot::channel();

        rt.spawn(async move {
            let _ = tx.send(manager.track_bytes(&id).await);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else { return };

            this.update(cx, |this, cx| {
                this.bytes_ready(
                    cx,
                    generation,
                    result
                        .map_err(EngineError::from)
                        .and_then(decode_to_samples),
                );
            })
            .ok();
        })
        .detach();
    }

    fn start_tick_loop(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK_INTERVAL).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn monoization(&self) -> Monoization {
        match self {
            Engine::Real(real) => real.monoization(),
        }
    }

    pub fn set_monoization(&mut self, state: Monoization) {
        match self {
            Engine::Real(real) => real.set_monoization(state),
        }
    }

    pub fn is_reverse(&self) -> bool {
        match self {
            Engine::Real(real) => real.is_reverse(),
        }
    }

    pub fn set_reverse(&mut self, state: bool) {
        match self {
            Engine::Real(real) => real.set_reverse(state),
        }
    }

    pub fn speed(&self) -> f32 {
        match self {
            Engine::Real(real) => real.speed(),
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        match self {
            Engine::Real(real) => real.set_speed(speed),
        }
    }

    pub fn reverb(&self) -> f32 {
        match self {
            Engine::Real(real) => real.reverb(),
        }
    }

    pub fn set_reverb(&mut self, reverb: f32) {
        match self {
            Engine::Real(real) => real.set_reverb(reverb),
        }
    }
}

pub fn fisher_yates(queue: &mut [Track], rng: &mut impl Rng) {
    for i in (1..queue.len()).rev() {
        let j = rng.random_range(0..i);
        queue.swap(i, j);
    }
}
