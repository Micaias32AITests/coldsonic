//! Real playback engine — same seam as the mock, rodio + library fetch.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use iced::Task;

use crate::Action;
use crate::library::Manager;

use super::player::AudioPlayer;
use super::queue::{Queue, QueueEntry};
use super::{EngineError, EngineEvent, EngineMsg, NowPlaying, PlaybackState, Track};

#[derive(Debug)]
pub struct RealEngine {
    manager: Arc<Manager>,
    player: AudioPlayer,
    queue: Queue,
    /// Bytes of the current track, kept so seek-fallback / Prev-restart can
    /// re-append without another fetch.
    current_bytes: Option<Bytes>,
    state: PlaybackState,
    volume: f32,
    /// Bumped on every fetch so a stale `BytesReady` cannot clobber a newer
    /// request (skip, new queue, stop).
    generation: u64,
    pending: VecDeque<EngineEvent>,
}

impl RealEngine {
    pub fn open(manager: Arc<Manager>) -> Self {
        Self {
            manager,
            player: AudioPlayer::open(),
            queue: Queue::new(),
            current_bytes: None,
            state: PlaybackState::Stopped,
            volume: 1.0,
            generation: 0,
            pending: VecDeque::new(),
        }
    }

    pub fn update(&mut self, msg: EngineMsg) -> Action<EngineMsg, EngineEvent> {
        match &msg {
            EngineMsg::BytesReady { generation, result } => {
                tracing::info!(
                    generation,
                    bytes = result.as_ref().ok().map(Bytes::len),
                    err = result.as_ref().err().map(ToString::to_string),
                    "engine msg BytesReady"
                );
            }
            other => tracing::info!(?other, "engine msg"),
        }
        match msg {
            EngineMsg::PlayQueue(tracks) => {
                self.player.stop();
                self.current_bytes = None;
                match self.queue.replace(tracks) {
                    Some(_) => return Action::task(self.start_fetch()),
                    None => {
                        self.state = PlaybackState::Stopped;
                    }
                }
            }
            EngineMsg::PlayNow(track) => {
                self.current_bytes = None;
                self.queue.play_now(track);
                return Action::task(self.start_fetch());
            }
            EngineMsg::Enqueue(track) => {
                self.queue.enqueue(track);
            }
            EngineMsg::Next => {
                self.current_bytes = None;
                self.player.stop();
                match self.queue.advance() {
                    Some(_) => return Action::task(self.start_fetch()),
                    None => {
                        self.state = PlaybackState::Stopped;
                        self.pending.push_back(EngineEvent::Stopped);
                    }
                }
            }
            EngineMsg::Prev => {
                if self.player.position() > Duration::from_secs(3) {
                    if let Some(entry) = self.queue.current() {
                        let current = entry.track.clone();
                        return self.restart_current(current);
                    }
                } else if self.queue.step_back().is_some() {
                    self.current_bytes = None;
                    return Action::task(self.start_fetch());
                }
            }
            EngineMsg::Pause => {
                if self.state == PlaybackState::Playing {
                    self.player.pause();
                    self.state = PlaybackState::Paused;
                    self.pending.push_back(EngineEvent::Paused);
                }
            }
            EngineMsg::Resume => {
                if self.state == PlaybackState::Paused {
                    self.player.resume();
                    self.state = PlaybackState::Playing;
                    self.pending.push_back(EngineEvent::Resumed);
                }
            }
            EngineMsg::Toggle => match self.state {
                PlaybackState::Playing => {
                    self.player.pause();
                    self.state = PlaybackState::Paused;
                    self.pending.push_back(EngineEvent::Paused);
                }
                PlaybackState::Paused => {
                    self.player.resume();
                    self.state = PlaybackState::Playing;
                    self.pending.push_back(EngineEvent::Resumed);
                }
                PlaybackState::Stopped | PlaybackState::Loading => {}
            },
            EngineMsg::Stop => {
                self.generation += 1;
                self.player.stop();
                self.queue.stop();
                self.current_bytes = None;
                if self.state != PlaybackState::Stopped {
                    self.state = PlaybackState::Stopped;
                    self.pending.push_back(EngineEvent::Stopped);
                }
            }
            EngineMsg::SetVolume(volume) => {
                self.volume = volume.clamp(0.0, 1.0);
                self.player.set_volume(self.volume);
                self.pending.push_back(EngineEvent::UpdateNowPlaying);
            }
            EngineMsg::Seek(position) => {
                if matches!(self.state, PlaybackState::Playing | PlaybackState::Paused) {
                    self.seek(position);
                    self.pending.push_back(EngineEvent::SeekDone);
                }
            }
            EngineMsg::BytesReady { generation, result } => {
                if generation != self.generation {
                    return Action::none();
                }
                match result {
                    Ok(bytes) => match self.play_ready(bytes) {
                        Ok(track) => {
                            self.pending.push_back(EngineEvent::PlaybackStarted(track));
                        }
                        Err(err) => {
                            self.fail(err.to_string());
                        }
                    },
                    Err(err) => self.fail(err.to_string()),
                }
            }
        }
        self.next_emit()
    }

    pub fn tick(&mut self) -> (Vec<EngineEvent>, Task<EngineMsg>) {
        let mut events: Vec<EngineEvent> = self.pending.drain(..).collect();
        let mut task = Task::none();

        if self.state == PlaybackState::Playing {
            events.push(EngineEvent::UpdateNowPlaying);
            if self.player.empty() {
                let finished = self
                    .queue
                    .current()
                    .expect("playing ⇒ current is set")
                    .track
                    .clone();
                events.push(EngineEvent::TrackFinished(finished));
                task = self.advance();
            }
        }

        (events, task)
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        let track = self.queue.current()?.track.clone();
        let position = match self.state {
            PlaybackState::Playing | PlaybackState::Paused => self.player.position(),
            PlaybackState::Loading | PlaybackState::Stopped => Duration::ZERO,
        };
        Some(NowPlaying {
            position,
            duration: track.duration,
            track,
            state: self.state,
            volume: self.volume,
        })
    }

    pub fn queue(&self) -> std::collections::vec_deque::Iter<'_, QueueEntry> {
        self.queue.upcoming()
    }

    fn start_fetch(&mut self) -> Task<EngineMsg> {
        let Some(entry) = self.queue.current() else {
            return Task::none();
        };
        let id = entry.track.id.clone();
        self.state = PlaybackState::Loading;
        self.current_bytes = None;
        self.player.stop();
        self.generation += 1;
        let generation = self.generation;
        let manager = self.manager.clone();
        Task::perform(
            async move { manager.track_bytes(&id).await },
            move |result| EngineMsg::BytesReady {
                generation,
                result: result.map_err(EngineError::from),
            },
        )
    }

    fn play_ready(&mut self, bytes: Bytes) -> Result<Track, EngineError> {
        let track = self
            .queue
            .current()
            .expect("BytesReady with matching generation ⇒ current is set")
            .track
            .clone();
        self.player.play_bytes(bytes.clone())?;
        self.player.set_volume(self.volume);
        self.current_bytes = Some(bytes);
        self.state = PlaybackState::Playing;
        Ok(track)
    }

    fn restart_current(&mut self, current: Track) -> Action<EngineMsg, EngineEvent> {
        if let Some(bytes) = self.current_bytes.clone() {
            match self.player.play_bytes(bytes) {
                Ok(()) => {
                    self.player.set_volume(self.volume);
                    self.state = PlaybackState::Playing;
                    self.pending
                        .push_back(EngineEvent::PlaybackStarted(current));
                    self.next_emit()
                }
                Err(err) => {
                    self.fail(err.to_string());
                    self.next_emit()
                }
            }
        } else {
            Action::task(self.start_fetch())
        }
    }

    fn seek(&mut self, position: Duration) {
        if self.player.try_seek(position) {
            return;
        }
        // Decoder wrapper chain sometimes refuses seek; re-append the same
        // bytes (from the start) and try once more. If that still fails we
        // stay at the beginning — the documented restart fallback.
        let Some(bytes) = self.current_bytes.clone() else {
            return;
        };
        if self.player.play_bytes(bytes).is_err() {
            return;
        }
        self.player.set_volume(self.volume);
        if self.state == PlaybackState::Paused {
            self.player.pause();
        }
        let _ = self.player.try_seek(position);
    }

    fn advance(&mut self) -> Task<EngineMsg> {
        self.current_bytes = None;
        match self.queue.advance() {
            Some(_) => self.start_fetch(),
            None => {
                self.player.stop();
                self.state = PlaybackState::Stopped;
                self.pending.push_back(EngineEvent::Stopped);
                Task::none()
            }
        }
    }

    fn fail(&mut self, message: String) {
        self.player.stop();
        self.queue.clear_current();
        self.current_bytes = None;
        self.state = PlaybackState::Stopped;
        self.pending.push_back(EngineEvent::Error(message));
        self.pending.push_back(EngineEvent::Stopped);
    }

    fn next_emit(&mut self) -> Action<EngineMsg, EngineEvent> {
        match self.pending.pop_front() {
            Some(event) => Action::emit(event),
            None => Action::none(),
        }
    }
}
