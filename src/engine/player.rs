//! Audio player (rodio 0.22) — device sink + [`Player`] behind a small API.
//!
//! The [`MixerDeviceSink`] owns the `cpal::Stream` and is `!Send`; it must
//! live on the UI thread. [`Player`] is `Send + Sync` and is what we actually
//! drive (append / pause / seek / position).

use std::fmt;
use std::io::Cursor;
use std::time::Duration;

use bytes::Bytes;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};

use super::EngineError;

pub struct AudioPlayer {
    /// Owns the output stream. Dropping this stops all sound.
    _sink: MixerDeviceSink,
    player: Player,
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
        Self {
            _sink: sink,
            player,
        }
    }

    /// Decode and play `bytes` as the (only) current source.
    pub fn play_bytes(&mut self, bytes: Bytes) -> Result<(), EngineError> {
        let len = bytes.len() as u64;
        let source = Decoder::builder()
            .with_data(Cursor::new(bytes))
            .with_byte_len(len)
            .build()
            .map_err(|err| EngineError::Decode(err.to_string()))?;
        self.player.stop();
        self.player.append(source);
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
        self.player.stop();
    }

    #[allow(dead_code)]
    pub fn skip(&mut self) {
        self.player.skip_one();
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.player.set_volume(volume.clamp(0.0, 1.0));
    }

    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }

    pub fn empty(&self) -> bool {
        self.player.empty()
    }

    /// Best-effort seek. Returns `false` when the current source cannot seek.
    pub fn try_seek(&mut self, pos: Duration) -> bool {
        self.player.try_seek(pos).is_ok()
    }
}
