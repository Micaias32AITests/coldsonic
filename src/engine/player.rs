//! Audio player (rodio 0.22) — device sink + [`Player`] behind a small API.
//!
//! The [`MixerDeviceSink`] owns the `cpal::Stream` and is `!Send`; it must
//! live on the UI thread. [`Player`] is `Send + Sync` and is what we actually
//! drive (append / pause / seek / position).

use std::io::Cursor;
use std::num::NonZeroU32;
use std::sync::atomic::{self, AtomicBool};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{fmt, sync::Arc};

use atomic_float::{AtomicF32, AtomicF64};
use bytes::Bytes;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};

use crate::engine::real::player_source::{PlayerCommunication, PlayerSource};
use crate::engine::{AtomicMonoization, AtomicRepeatMode};

use super::{EngineError, Monoization, RepeatMode};

pub struct AudioPlayer {
    /// Owns the output stream. Dropping this stops all sound.
    _sink: MixerDeviceSink,
    player: Player,
    coms: Arc<PlayerCommunication>,
    tx: mpsc::Sender<TrackSamples>,
}

impl fmt::Debug for AudioPlayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioPlayer")
            .field("len", &self.player.len())
            .field("pos", &self.player.get_pos())
            .field("empty", &self.player.empty())
            .finish_non_exhaustive()
    }
}

impl AudioPlayer {
    pub fn open() -> Self {
        let mut sink = DeviceSinkBuilder::open_default_sink().expect("open default audio output");
        sink.log_on_drop(false);
        let player = Player::connect_new(sink.mixer());
        player.set_volume(1.0);

        let coms = Arc::new(PlayerCommunication {
            speed: AtomicF32::new(1.),
            reverb: AtomicF32::new(0.),
            reverse: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            repeat: AtomicRepeatMode::new(RepeatMode::None),
            pos_secs: AtomicF64::new(0.),
            mono: AtomicMonoization::new(Monoization::None),
        });
        let (tx, rx) = mpsc::channel();

        player.append(PlayerSource::new(TrackSamples::default(), coms.clone(), rx));
        player.play();

        Self {
            _sink: sink,
            player,
            coms,
            tx,
        }
    }

    /// play `track` as the (only) current source.
    pub fn play_track(&mut self, track: TrackSamples) -> Result<(), EngineError> {
        self.coms.finished.store(false, atomic::Ordering::Relaxed);
        let _ = self.tx.send(track);
        self.player.play();

        Ok(())
    }

    pub fn pause(&mut self) {
        self.player.pause();
    }

    pub fn resume(&mut self) {
        self.player.play();
    }

    pub fn stop(&mut self) {
        // cool fuckass trick
        let _ = self.tx.send(TrackSamples::default());
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.player.set_volume(volume.clamp(0.0, 1.0));
    }

    pub fn position(&self) -> Duration {
        let sex = self.coms.pos_secs.load(atomic::Ordering::Relaxed);
        Duration::from_secs_f64(sex)
    }

    pub fn is_reverse(&self) -> bool {
        self.coms.reverse.load(atomic::Ordering::Relaxed)
    }

    pub fn set_reverse(&mut self, state: bool) {
        self.coms.reverse.store(state, atomic::Ordering::Relaxed)
    }

    pub fn empty(&self) -> bool {
        self.coms.finished.load(atomic::Ordering::Relaxed)
    }

    /// Best-effort seek. Returns `false` when the current source cannot seek.
    pub fn try_seek(&mut self, pos: Duration) -> bool {
        self.player.try_seek(pos).is_ok()
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.coms.speed.store(speed, atomic::Ordering::Relaxed);
    }

    pub fn set_reverb(&mut self, reverb: f32) {
        self.coms.reverb.store(reverb, atomic::Ordering::Relaxed);
    }

    pub fn reverb(&self) -> f32 {
        self.coms.reverb.load(atomic::Ordering::Relaxed)
    }

    pub fn speed(&self) -> f32 {
        self.coms.speed.load(atomic::Ordering::Relaxed)
    }

    pub fn monoization(&self) -> Monoization {
        self.coms.mono.load(atomic::Ordering::Relaxed)
    }

    pub fn set_monoization(&mut self, monoization: Monoization) {
        self.coms.mono.store(monoization, atomic::Ordering::Relaxed);
    }

    pub fn repeat(&self) -> RepeatMode {
        self.coms.repeat.load(atomic::Ordering::Relaxed)
    }

    pub fn set_repeat(&self, repeat: RepeatMode) {
        self.coms.repeat.store(repeat, atomic::Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct TrackSamples {
    pub samples: Vec<f32>,
    pub rate: NonZeroU32,
}

impl Default for TrackSamples {
    fn default() -> Self {
        Self {
            samples: Default::default(),
            rate: NonZeroU32::MIN,
        }
    }
}

pub fn decode_to_samples(bytes: Bytes) -> Result<TrackSamples, EngineError> {
    let t = Instant::now();
    tracing::info!("start decoding");
    let decoded = Decoder::builder()
        .with_byte_len(bytes.len() as u64)
        .with_data(Cursor::new(bytes))
        .build()
        .map_err(|e| EngineError::Decode(e.to_string()))?;
    tracing::info!(t = t.elapsed().as_secs_f64(), "created decoder");

    let rate = decoded.sample_rate();
    let ch = u16::from(decoded.channels()) as usize;
    let raw: Vec<f32> = decoded.collect();

    tracing::info!(t = t.elapsed().as_secs_f64(), "got info");
    let samples = if ch == 2 {
        raw
    } else {
        tracing::info!(
            t = t.elapsed().as_secs_f64(),
            ch = ch,
            rate = rate,
            "needs to be recomputed"
        );
        let mut out = Vec::with_capacity(raw.len() / ch.max(1) * 2);
        for frame in raw.chunks_exact(ch.max(1)) {
            out.push(frame[0]);
            out.push(if ch > 1 { frame[1] } else { frame[0] });
        }
        tracing::info!(t = t.elapsed().as_secs_f64(), "done recomputing");
        out
    };

    tracing::info!(t = t.elapsed().as_secs_f64(), "done");
    Ok(TrackSamples { samples, rate })
}
