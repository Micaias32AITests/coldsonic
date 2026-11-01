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

use super::queue::{Queue, QueueEntry};
use super::{EngineEvent, EngineMsg, NowPlaying, PlaybackState};

/// Fake playback speed: each 250 ms tick advances the position a second so the
/// mock bar visibly moves.
const SIMULATED_SECONDS_PER_TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct MockEngine {
    queue: Queue,
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
            queue: Queue::new(),
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
                self.position = Duration::ZERO;
                match self.queue.replace(tracks) {
                    Some(entry) => {
                        let track = entry.track.clone();
                        self.state = PlaybackState::Playing;
                        self.pending.push_back(EngineEvent::PlaybackStarted(track));
                    }
                    None => {
                        self.state = PlaybackState::Stopped;
                    }
                }
            }
            EngineMsg::PlayNow(track) => {
                let track = self.queue.play_now(track).track.clone();
                self.position = Duration::ZERO;
                self.state = PlaybackState::Playing;
                self.pending.push_back(EngineEvent::PlaybackStarted(track));
            }
            EngineMsg::Enqueue(track) => {
                self.queue.enqueue(track);
            }
            EngineMsg::Next => {
                self.position = Duration::ZERO;
                match self.queue.advance() {
                    Some(entry) => {
                        let track = entry.track.clone();
                        self.state = PlaybackState::Playing;
                        self.pending.push_back(EngineEvent::PlaybackStarted(track));
                    }
                    None => {
                        self.state = PlaybackState::Stopped;
                        self.pending.push_back(EngineEvent::Stopped);
                    }
                }
            }
            EngineMsg::Prev => {
                if self.position > Duration::from_secs(3) {
                    // Past the restart threshold → restart the current track.
                    if let Some(entry) = self.queue.current() {
                        let track = entry.track.clone();
                        self.position = Duration::ZERO;
                        self.state = PlaybackState::Playing;
                        self.pending.push_back(EngineEvent::PlaybackStarted(track));
                    }
                } else if let Some(entry) = self.queue.step_back() {
                    let track = entry.track.clone();
                    self.position = Duration::ZERO;
                    self.state = PlaybackState::Playing;
                    self.pending.push_back(EngineEvent::PlaybackStarted(track));
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
                self.queue.stop();
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
                    .queue
                    .current()
                    .map(|entry| entry.track.duration)
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
                .queue
                .current()
                .is_some_and(|entry| entry.track.duration <= self.position)
            {
                let finished = self
                    .queue
                    .current()
                    .expect("playing ⇒ current is set")
                    .track
                    .clone();
                events.push(EngineEvent::TrackFinished(finished));
                self.advance();
            }
        }

        (events, Task::none())
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        let track = self.queue.current()?.track.clone();
        Some(NowPlaying {
            position: self.position,
            duration: track.duration,
            track,
            state: self.state,
            volume: self.volume,
        })
    }

    pub fn queue(&self) -> std::collections::vec_deque::Iter<'_, QueueEntry> {
        self.queue.upcoming()
    }

    /// Auto-advance to the next queued track (or stop when the queue is empty).
    fn advance(&mut self) {
        match self.queue.advance() {
            Some(entry) => {
                let track = entry.track.clone();
                self.position = Duration::ZERO;
                self.pending.push_back(EngineEvent::PlaybackStarted(track));
            }
            None => {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::library::TrackId;

    use super::*;
    use crate::engine::Track;

    fn track(name: &str) -> Track {
        Track {
            id: TrackId::new(name),
            title: name.to_owned(),
            artist: None,
            album: None,
            track_no: None,
            duration: Duration::from_secs(60),
            cover: None,
        }
    }

    fn upcoming(engine: &MockEngine) -> Vec<&str> {
        engine
            .queue()
            .map(|entry| entry.track.title.as_str())
            .collect()
    }

    fn current(engine: &MockEngine) -> Option<&str> {
        engine
            .queue
            .current()
            .map(|entry| entry.track.title.as_str())
    }

    #[test]
    fn play_now_does_not_touch_upcoming() {
        let mut engine = MockEngine::new();
        engine.update(EngineMsg::PlayQueue(vec![track("a"), track("b")]));

        engine.update(EngineMsg::PlayNow(track("c")));

        assert_eq!(current(&engine), Some("c"));
        assert_eq!(upcoming(&engine), ["b"]);
    }

    #[test]
    fn enqueue_appends_and_keeps_current_playing() {
        let mut engine = MockEngine::new();
        engine.update(EngineMsg::PlayQueue(vec![track("a")]));

        engine.update(EngineMsg::Enqueue(track("b")));

        assert_eq!(current(&engine), Some("a"));
        assert_eq!(upcoming(&engine), ["b"]);
        assert_eq!(engine.state, PlaybackState::Playing);
    }

    #[test]
    fn prev_steps_back_when_near_the_start() {
        let mut engine = MockEngine::new();
        engine.update(EngineMsg::PlayQueue(vec![track("a"), track("b")]));
        engine.update(EngineMsg::Next);

        engine.update(EngineMsg::Prev);

        assert_eq!(current(&engine), Some("a"));
        assert_eq!(upcoming(&engine), ["b"]);
    }

    #[test]
    fn prev_restarts_without_mutating_the_queue_past_the_threshold() {
        let mut engine = MockEngine::new();
        engine.update(EngineMsg::PlayQueue(vec![track("a"), track("b")]));
        engine.update(EngineMsg::Next);
        let before: Vec<String> = upcoming(&engine).into_iter().map(str::to_owned).collect();
        engine.position = Duration::from_secs(10);

        engine.update(EngineMsg::Prev);

        assert_eq!(current(&engine), Some("b"));
        let after: Vec<String> = upcoming(&engine).into_iter().map(str::to_owned).collect();
        assert_eq!(before, after);
        assert_eq!(engine.position, Duration::ZERO);
    }
}
