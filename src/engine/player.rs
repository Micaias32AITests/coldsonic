//! Audio player (rodio) — wraps the device sink + player behind a small API.

use std::io::Cursor;
use std::time::Duration;

use bytes::Bytes;
use rodio::{
    Decoder, DeviceSink, DeviceSinkBuilder, Mixer, Player, OutputStreamBuilder, Sample, Source,
};
use rodio::cpal::traits::HostTrait;

#[derive(Debug)]
pub struct AudioPlayer {
    _stream: rodio::OutputStream,
    _mixer: Mixer,
    sink: DeviceSink,
    player: Player,
}

impl AudioPlayer {
    pub fn open() -> Self {
        let (stream, stream_handle) = OutputStreamBuilder::open_default_stream()
            .expect("open default audio stream");
        let mixer = Mixer::default();
        stream_handle
            .connect_to_output(&mixer)
            .expect("connect mixer to output stream");
        let sink = rodio::Sink::new(stream_handle);
        // Use DeviceSink style via Player connected through mixer? Easier: build player
        // But simpler approach using rodio's high-level API
        // Alternatively, use Sink directly
        let _ = sink;
        // Simpler: use rodio::Sink
        let sink = rodio::Sink::try_new(stream_handle).expect("create sink");
        Self {
            _stream: stream,
            _mixer: mixer,
            sink,
            player: rodio::Player::new(mixer),
        }
    }

    pub fn play_bytes(&mut self, bytes: Bytes) {
        let source = Decoder::new(Cursor::new(bytes)).expect("decode audio");
        self.sink.append(source);
        self.sink.play();
    }

    pub fn pause(&mut self) {
        self.sink.pause();
    }

    pub fn resume(&mut self) {
        self.sink.play();
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.sink.clear();
    }

    pub fn clear(&mut self) {
        self.sink.clear();
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.sink.set_volume(volume.clamp(0.0, 1.0));
    }

    pub fn position(&self) -> Duration {
        // rodio's Sink doesn't track position easily in all cases; for now approximate
        Duration::ZERO
    }

    pub fn empty(&self) -> bool {
        self.sink.empty()
    }

    pub fn try_seek(&mut self, _pos: Duration) -> bool {
        false
    }
}
