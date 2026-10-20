//! Mock engine — same seam, no audio.
//!
//! Facet 1 validates the intent → event → coordinator → view pipe before any
//! audio exists: every incoming [`EngineMsg`] is logged, the queue is faked,
//! and `tick` advances a fake position (~1 s per tick) so the bar visibly
//! moves and tracks auto-advance.

use std::collections::VecDeque;
use std::time::Duration;

use iced::Task;

use crate::Action;

use super::{EngineEvent, EngineMsg, NowPlaying, PlaybackState, Track};

/// Fake playback speed: each 250 ms tick advances the position a second so the
/// mock bar visibly moves.
const SIMULATED_SECONDS_PER_TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct MockEngine {
    queue: VecDeque<Track>,
    /// Played tracks, newest last — feeds `Prev`.
    history: VecDeque<Track>,
    current: Option<Track>,
    position: Duration,
    state: PlaybackState,
    /// Engine-owned volume (0.0–1.0), read by the bar from the snapshot.
    volume: f32,
    /// Events produced by `update` that couldn't all ride the single
    /// `Action::Emit` in one call; drained on subsequent `tick`s.
    pending: VecDeque<EngineEvent>,
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEngine {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            history: VecDeque::new(),
            current: None,
            position: Duration::ZERO,
            state: PlaybackState::Stopped,
            volume: 1.0,
            pending: VecDeque::new(),
        }
    }

    pub fn update(&mut self, msg: EngineMsg) -> Action<EngineMsg, EngineEvent> {
        tracing::info!(?msg, "engine msg");
        match msg {
            EngineMsg::PlayQueue(tracks) => {
                self.queue = tracks.into();
                self.history.clear();
                self.position = Duration::ZERO;
                match self.queue.pop_front() {
                    Some(track) => {
                        self.current = Some(track.clone());
                        self.state = PlaybackState::Playing;
                        self.pending.push_back(EngineEvent::QueueChanged);
                        self.pending.push_back(EngineEvent::PlaybackStarted(track));
                    }
                    None => {
                        self.current = None;
                        self.state = PlaybackState::Stopped;
                        self.pending.push_back(EngineEvent::QueueChanged);
                    }
                }
            }
            EngineMsg::PlayNext(track) => {
                if let Some(current) = self.current.take() {
                    self.history.push_back(current);
                }
                self.queue.push_front(track.clone());
                self.current = Some(track.clone());
                self.position = Duration::ZERO;
                self.state = PlaybackState::Playing;
                self.pending.push_back(EngineEvent::PlaybackStarted(track));
                self.pending.push_back(EngineEvent::QueueChanged);
            }
            EngineMsg::Enqueue(track) => {
                self.queue.push_back(track);
                self.pending.push_back(EngineEvent::QueueChanged);
            }
            EngineMsg::Next => {
                if let Some(current) = self.current.take() {
                    self.history.push_back(current);
                }
                match self.queue.pop_front() {
                    Some(track) => {
                        self.current = Some(track.clone());
                        self.position = Duration::ZERO;
                        self.state = PlaybackState::Playing;
                        self.pending.push_back(EngineEvent::PlaybackStarted(track));
                    }
                    None => {
                        self.current = None;
                        self.position = Duration::ZERO;
                        self.state = PlaybackState::Stopped;
                        self.pending.push_back(EngineEvent::Stopped);
                    }
                }
            }
            EngineMsg::Prev => {
                if self.position > Duration::from_secs(3) {
                    // Past the restart threshold → restart the current track.
                    if let Some(current) = self.current.clone() {
                        self.position = Duration::ZERO;
                        self.state = PlaybackState::Playing;
                        self.pending
                            .push_back(EngineEvent::PlaybackStarted(current));
                    }
                } else if let Some(prev) = self.history.pop_back() {
                    // Step back into history; the current track heads to the
                    // queue front so `Next` replays it.
                    if let Some(current) = self.current.take() {
                        self.queue.push_front(current);
                    }
                    self.current = Some(prev.clone());
                    self.position = Duration::ZERO;
                    self.state = PlaybackState::Playing;
                    self.pending.push_back(EngineEvent::PlaybackStarted(prev));
                }
            }
            EngineMsg::Pause => {
                if self.state == PlaybackState::Playing {
                    self.state = PlaybackState::Paused;
                    self.pending.push_back(EngineEvent::Paused);
                }
            }
            EngineMsg::Resume => {
                if self.state == PlaybackState::Paused {
                    self.state = PlaybackState::Playing;
                    self.pending.push_back(EngineEvent::Resumed);
                }
            }
            EngineMsg::Toggle => match self.state {
                PlaybackState::Playing => {
                    self.state = PlaybackState::Paused;
                    self.pending.push_back(EngineEvent::Paused);
                }
                PlaybackState::Paused => {
                    self.state = PlaybackState::Playing;
                    self.pending.push_back(EngineEvent::Resumed);
                }
                PlaybackState::Stopped | PlaybackState::Loading => {}
            },
            EngineMsg::Stop => {
                self.current = None;
                self.history.clear();
                self.position = Duration::ZERO;
                if self.state != PlaybackState::Stopped {
                    self.state = PlaybackState::Stopped;
                    self.pending.push_back(EngineEvent::Stopped);
                }
            }
            // The engine owns volume; push a snapshot refresh so the bar's
            // volume slider updates while dragging.
            EngineMsg::SetVolume(volume) => {
                self.volume = volume.clamp(0.0, 1.0);
                self.pending.push_back(EngineEvent::UpdateNowPlaying);
            }
            EngineMsg::Seek(position) => {
                let max = self
                    .current
                    .as_ref()
                    .map(|track| track.duration)
                    .unwrap_or(Duration::from_secs(10));
                self.position = position.min(max);
                self.pending.push_back(EngineEvent::SeekDone);
            }
            EngineMsg::BytesReady { .. } => {}
        }
        self.next_emit()
    }

    pub fn tick(&mut self) -> (Vec<EngineEvent>, Task<EngineMsg>) {
        let mut events: Vec<EngineEvent> = self.pending.drain(..).collect();

        if self.state == PlaybackState::Playing {
            self.position += SIMULATED_SECONDS_PER_TICK;
            events.push(EngineEvent::UpdateNowPlaying);
            if self
                .current
                .as_ref()
                .is_some_and(|t| t.duration <= self.position)
            {
                let finished = self.current.clone().expect("playing ⇒ current is set");
                events.push(EngineEvent::TrackFinished(finished));
                self.advance();
            }
        }

        (events, Task::none())
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        let track = self.current.clone()?;
        Some(NowPlaying {
            position: self.position,
            duration: track.duration,
            track,
            state: self.state,
            volume: self.volume,
        })
    }

    /// Auto-advance to the next queued track (or stop when the queue is empty).
    fn advance(&mut self) {
        if let Some(current) = self.current.take() {
            self.history.push_back(current);
        }
        match self.queue.pop_front() {
            Some(next) => {
                self.current = Some(next.clone());
                self.position = Duration::ZERO;
                self.pending.push_back(EngineEvent::PlaybackStarted(next));
            }
            None => {
                self.current = None;
                self.position = Duration::ZERO;
                self.state = PlaybackState::Stopped;
                self.pending.push_back(EngineEvent::Stopped);
            }
        }
    }

    /// Pop one pending event for the single `Action::Emit` slot.
    fn next_emit(&mut self) -> Action<EngineMsg, EngineEvent> {
        match self.pending.pop_front() {
            Some(event) => Action::emit(event),
            None => Action::none(),
        }
    }
}
