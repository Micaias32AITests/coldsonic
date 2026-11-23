//! Player engine: the seam between the UI and playback.
//!
//! The engine never talks to screens directly — it emits [`EngineEvent`]s
//! upward to its owner (`MainScreen`, the coordinator), which pushes state
//! down to the views. Screens raise *intents* ([`EngineMsg`]) and read pushed
//! state; they never touch audio.

pub mod mock;
pub mod player;
pub mod queue;
pub mod real;

use std::time::Duration;

use crate::library::{ManagerHandle, TrackId};

use bytes::Bytes;
use gpui_kit::{App, Context, Entity, Global};
pub use queue::{Queue, QueueEntry, QueueEntryId};

/// What we play — the neutral track every provider hands over.
pub type Track = crate::library::Track;

/// Commands from the UI into the engine.
#[derive(Debug, Clone)]
pub enum EngineMsg {
    /// Replace the queue, play the first track.
    PlayQueue(Vec<Track>),
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
    BytesReady {
        generation: u64,
        result: Result<bytes::Bytes, EngineError>,
    },
}

/// Events the engine pushes up to its owner. `MainScreen` is the coordinator
/// today; a player screen later.
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

/// What the engine asks its owner to do after [`Engine::update`] or
/// [`Engine::tick`].
///
/// The engine itself never touches a runtime: async work leaves as a request
/// and comes back as an [`EngineMsg`], so the UI layer decides how to run it.
#[derive(Debug, Clone, Default)]
pub enum EngineResponse {
    /// Nothing to do.
    #[default]
    Nothing,
    /// An event to push to the UI.
    Event(EngineEvent),
    /// The engine needs the bytes of `id` before playback can start. The owner
    /// fetches them and feeds `EngineMsg::BytesReady { generation, .. }` back
    /// into [`Engine::update`].
    LoadTrack { generation: u64, id: TrackId },
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
    /// 0.0–1.0. The engine owns volume; the UI only reads the snapshot.
    pub volume: f32,
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

/// The engine itself. Not `Clone`: the real variant owns a `!Send` device
/// sink.
#[derive(Debug)]
pub enum Engine {
    Real(real::RealEngine),
}

#[derive(Clone)]
pub struct EngineHandle(pub Entity<Engine>);
impl Global for EngineHandle {}

impl EngineHandle {
    pub fn get<T>(cx: &App) -> Entity<Engine> {
        cx.global::<EngineHandle>().0.clone()
    }
}

impl Engine {
    pub fn real(cx: &mut Context<Self>, m_handle: ManagerHandle) -> Self {
        Self::Real(real::RealEngine::open(m_handle))
    }

    /// Route a UI command into the engine. The returned [`EngineResponse`]
    /// carries any immediate [`EngineEvent`] and any work the owner must run.
    pub fn update(&mut self, msg: EngineMsg) -> EngineResponse {
        match self {
            Engine::Real(engine) => engine.update(msg),
        }
    }

    pub fn play_queue(&mut self, cx: &mut Context<Self>, queue: Vec<Track>) {
        todo!()
    }

    pub fn play_now(&mut self, cx: &mut Context<Self>, track: Track) {
        todo!()
    }

    pub fn enqueue(&mut self, cx: &mut Context<Self>, track: Track) {
        todo!()
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    pub fn prev(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    pub fn set_volume(&mut self, cx: &mut Context<Self>, volume: f32) {
        todo!()
    }

    pub fn seek(&mut self, cx: &mut Context<Self>, duration: Duration) {
        todo!()
    }

    fn bytes_ready(&mut self, cx: &mut Context<Self>, bytes: Bytes) {
        todo!()
    }

    /// Called by the shell on each tick. Events are pushed to the UI; the
    /// response carries a fetch kicked off by auto-advance.
    pub fn tick(&mut self) -> (Vec<EngineEvent>, EngineResponse) {
        match self {
            Engine::Real(engine) => engine.tick(),
        }
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        match self {
            Engine::Real(engine) => engine.now_playing(),
        }
    }

    /// Borrowed iteration over the upcoming queue, in playback order.
    ///
    /// There is deliberately no mutable accessor: mutations go through
    /// [`EngineMsg`].
    pub fn queue(&self) -> std::collections::vec_deque::Iter<'_, QueueEntry> {
        match self {
            Engine::Real(engine) => engine.queue(),
        }
    }
}
