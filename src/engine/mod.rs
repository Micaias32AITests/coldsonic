//! Player engine: the seam between the UI and playback.
//!
//! The engine never talks to screens directly — it emits [`EngineEvent`]s
//! upward to its owner (`MainScreen`, the coordinator), which pushes state
//! down to the views. Screens raise *intents* ([`EngineMsg`]) and read pushed
//! state; they never touch audio.
//!
//! Facet 1 ships a mock engine behind this seam; the real one (rodio) arrives
//! in Facet 2 behind the same interface.

pub mod mock;

use std::time::Duration;

use crate::Action;

/// What we play. `Child` already carries `id` (fetching), `duration`,
/// `title`/`artist`/`album`, `cover_art` — everything we need.
pub type Track = opensubsonic::data::Child;

/// Commands from the UI into the engine.
#[derive(Debug, Clone)]
pub enum EngineMsg {
    /// Replace the queue, play the first track.
    PlayQueue(Vec<Track>),
    /// Play now, insert at the front of the queue.
    PlayNext(Track),
    /// Append to the queue end.
    Enqueue(Track),
    /// Skip to the next queued track.
    Next,
    /// Go back: restart the current track when it's past the threshold,
    /// otherwise step back into the playback history.
    Prev,
    Pause,
    Resume,
    Toggle,
    Stop,
    SetVolume(f32),
    /// Facet 2+ (best-effort).
    Seek(Duration), // NOTE: Facet 2 adds `BytesReady { generation, result }`
                    // — deferred: its `Result<Bytes, EngineError>` breaks the
                    // `Clone` bound iced's buttons need on `Msg`, and Facet 1
                    // has no fetching anyway.
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
    QueueChanged,
    Error(String),
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

/// Errors surfaced by the engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("subsonic error: {0}")]
    Subsonic(#[from] opensubsonic::Error),
}

/// The engine itself. An enum wrapper so `MainScreen` keeps on the same seam
/// (`#[derive(Debug, Clone)]`) once the real engine lands in Facet 2.
#[derive(Debug, Clone)]
pub enum Engine {
    Mock(mock::MockEngine),
}

impl Engine {
    pub fn mock() -> Self {
        Self::Mock(mock::MockEngine::new())
    }

    /// Route a UI command into the engine. The returned
    /// [`Action`](crate::Action) carries any immediate [`EngineEvent`].
    pub fn update(&mut self, msg: EngineMsg) -> Action<EngineMsg, EngineEvent> {
        match self {
            Engine::Mock(engine) => engine.update(msg),
        }
    }

    /// Called by `MainScreen` on each tick. Returns events to push to the UI.
    pub fn tick(&mut self) -> Vec<EngineEvent> {
        match self {
            Engine::Mock(engine) => engine.tick(),
        }
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        match self {
            Engine::Mock(engine) => engine.now_playing(),
        }
    }
}
