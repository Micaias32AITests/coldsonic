//! Real playback engine — same seam as the mock, rodio + library fetch.

use std::collections::VecDeque;
use std::time::Duration;

use bytes::Bytes;

use crate::library::ManagerHandle;

use super::player::AudioPlayer;
use super::queue::{Queue, QueueEntry};
use super::{
    EngineError, EngineEvent, EngineMsg, EngineResponse, NowPlaying, PlaybackState, Track,
};

#[derive(Debug)]
pub struct RealEngine {
    player: AudioPlayer,
    queue: Queue,
    manager: ManagerHandle,
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
    pub fn open(manager: ManagerHandle) -> Self {
        Self {
            player: AudioPlayer::open(),
            queue: Queue::new(),
            manager,
            current_bytes: None,
            state: PlaybackState::Stopped,
            volume: 1.0,
            generation: 0,
            pending: VecDeque::new(),
        }
    }

    /// The library handle the entity uses to fetch track bytes.
    pub fn manager(&self) -> &ManagerHandle {
        &self.manager
    }

    pub fn update(&mut self, msg: EngineMsg) -> EngineResponse {
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
            EngineMsg::PlayQueue(tracks, pos) => {
                self.player.stop();
                self.current_bytes = None;
                match self.queue.replace(tracks, pos) {
                    Some(_) => return self.start_fetch(),
                    None => {
                        self.state = PlaybackState::Stopped;
                    }
                }
            }
            EngineMsg::PlayNow(track) => {
                self.current_bytes = None;
                self.queue.play_now(track);
                return self.start_fetch();
            }
            EngineMsg::Enqueue(track) => {
                self.queue.enqueue(track);
            }
            EngineMsg::Next => {
                self.current_bytes = None;
                self.player.stop();
                match self.queue.advance() {
                    Some(_) => return self.start_fetch(),
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
                    return self.start_fetch();
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
                    return EngineResponse::Nothing;
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

    pub fn tick(&mut self) -> (Vec<EngineEvent>, EngineResponse) {
        let mut events: Vec<EngineEvent> = self.pending.drain(..).collect();
        let mut response = EngineResponse::Nothing;

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
                response = self.advance();
            }
        }

        (events, response)
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
        })
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn queue(&self) -> std::collections::vec_deque::Iter<'_, QueueEntry> {
        self.queue.upcoming()
    }

    /// Ask the owner for the current track's bytes. The reply is
    /// [`EngineMsg::BytesReady`] with the same `generation`.
    fn start_fetch(&mut self) -> EngineResponse {
        let Some(entry) = self.queue.current() else {
            return EngineResponse::Nothing;
        };
        let id = entry.track.id.clone();
        self.state = PlaybackState::Loading;
        self.current_bytes = None;
        self.player.stop();
        self.generation += 1;
        EngineResponse::LoadTrack {
            generation: self.generation,
            id,
        }
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

    fn restart_current(&mut self, current: Track) -> EngineResponse {
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
            self.start_fetch()
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

    fn advance(&mut self) -> EngineResponse {
        self.current_bytes = None;
        match self.queue.advance() {
            Some(_) => self.start_fetch(),
            None => {
                self.player.stop();
                self.state = PlaybackState::Stopped;
                self.pending.push_back(EngineEvent::Stopped);
                EngineResponse::Nothing
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

    fn next_emit(&mut self) -> EngineResponse {
        match self.pending.pop_front() {
            Some(event) => EngineResponse::Event(event),
            None => EngineResponse::Nothing,
        }
    }
}
