use std::sync::{
    Arc,
    atomic::{self, AtomicBool},
    mpsc,
};

use atomic_float::{AtomicF32, AtomicF64};
use num_traits::ToPrimitive;
use rodio::{ChannelCount, SampleRate, Source};

use crate::engine::{
    AtomicMonoization, AtomicRepeatMode, Monoization, RepeatMode, player::TrackSamples,
};

pub const RATE: usize = 48_000;
pub const RATE_F64: f64 = RATE as f64;
pub const CHANNELS: usize = 2;

pub struct PlayerCommunication {
    pub reverse: AtomicBool,
    pub speed: AtomicF32,
    pub reverb: AtomicF32,
    pub finished: AtomicBool,
    pub mono: AtomicMonoization,
    pub repeat: AtomicRepeatMode,
    pub pos_secs: AtomicF64,
}

pub struct PlayerSource {
    buf: TrackSamples,
    cursor: f64, // for speed changes, it will need to become an f64,
    out: [f32; CHANNELS],
    next_sample: NextSample,
    coms: Arc<PlayerCommunication>,
    new_song: mpsc::Receiver<TrackSamples>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NextSample {
    Left,
    Right,
    NewFrame,
}

impl PlayerSource {
    pub fn new(
        buf: TrackSamples,
        coms: Arc<PlayerCommunication>,
        rec: mpsc::Receiver<TrackSamples>,
    ) -> Self {
        Self {
            buf,
            coms,
            cursor: 0.,
            out: [0.; CHANNELS],
            next_sample: NextSample::NewFrame,
            new_song: rec,
        }
    }
}

impl Iterator for PlayerSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_sample {
            NextSample::Left => {
                self.next_sample = NextSample::Right;
                Some(self.out[0])
            }
            NextSample::Right => {
                self.next_sample = NextSample::NewFrame;
                Some(self.out[1])
            }
            NextSample::NewFrame => {
                if let Ok(buf) = self.new_song.try_recv() {
                    let frames = (buf.samples.len() / CHANNELS) as f64;

                    self.buf = buf;

                    self.cursor = if self.coms.reverse.load(atomic::Ordering::Relaxed) {
                        frames - 1.
                    } else {
                        0.
                    };

                    self.coms
                        .pos_secs
                        .store(self.cursor, atomic::Ordering::Relaxed);

                    self.coms.finished.store(false, atomic::Ordering::Relaxed);
                }

                let frames_usize = self.buf.samples.len() / CHANNELS;
                let frames = frames_usize as f64;
                let track_rate = u32::from(self.buf.rate) as f64;

                let Some([mut l, mut r]) = get_frame(&self.buf, self.cursor) else {
                    if !self.buf.samples.is_empty() {
                        if self.coms.repeat.load(atomic::Ordering::Relaxed) == RepeatMode::One {
                            let frames = (self.buf.samples.len() / CHANNELS) as f64;

                            self.cursor = if self.coms.reverse.load(atomic::Ordering::Relaxed) {
                                frames - 1.
                            } else {
                                0.
                            };

                            self.coms
                                .pos_secs
                                .store(self.cursor, atomic::Ordering::Relaxed);

                            return self.next();
                        } else {
                            self.buf.samples.clear();
                            self.coms.finished.store(true, atomic::Ordering::Relaxed);
                        }
                    }

                    self.out = [0.; CHANNELS];
                    self.next_sample = NextSample::Right;
                    return Some(0.);
                };

                match self.coms.mono.load(atomic::Ordering::Relaxed) {
                    Monoization::None => (),
                    Monoization::Mix => {
                        const MONO_GAIN: f32 = 1.2;

                        // in the mono step, after get_frame:
                        let m = ((l + r) * 0.5 * MONO_GAIN).clamp(-1.0, 1.0);

                        l = m;
                        r = m;
                    }
                    Monoization::OnlyLeft => {
                        r = l;
                    }
                    Monoization::OnlyRight => {
                        l = r;
                    }
                }

                self.out = [l, r];

                let advance =
                    track_rate / RATE_F64 * self.coms.speed.load(atomic::Ordering::Relaxed) as f64;
                if self.coms.reverse.load(atomic::Ordering::Relaxed) {
                    if self.cursor == 0. {
                        self.cursor = frames;
                    } else {
                        self.cursor -= advance;
                        self.coms
                            .pos_secs
                            .store(self.cursor / track_rate, atomic::Ordering::Relaxed);
                    }
                } else {
                    self.cursor += advance;
                    self.coms
                        .pos_secs
                        .store(self.cursor / track_rate, atomic::Ordering::Relaxed);
                }

                self.next_sample = NextSample::Right;
                Some(self.out[0])
            }
        }
    }
}

impl Source for PlayerSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        ChannelCount::new(CHANNELS as u16).expect("unreachable")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        SampleRate::new(RATE as u32).expect("unrechable")
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }

    fn try_seek(&mut self, pos: std::time::Duration) -> Result<(), rodio::source::SeekError> {
        let track_rate = u32::from(self.buf.rate) as f64;

        self.cursor = pos.as_secs_f64() * track_rate;
        self.coms
            .pos_secs
            .store(pos.as_secs_f64(), atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[inline]
fn get_frame(track: &TrackSamples, frame: f64) -> Option<[f32; CHANNELS]> {
    let frame_pos_min = frame.floor().to_usize()?.checked_mul(CHANNELS)?;
    let frame_pos_max = frame.ceil().to_usize()?.checked_mul(CHANNELS)?;

    let &[l_l, l_r] = track.samples.get(frame_pos_min..=frame_pos_min + 1)? else {
        // currently, the compiler is kind of dumb, so we need this.
        return None;
    };

    match track.samples.get(frame_pos_max..=frame_pos_max + 1) {
        Some(&[r_l, r_r]) => {
            // interplolate
            let t = frame.fract() as f32;

            let l = l_l + (r_l - l_l) * t;
            let r = l_r + (r_r - l_r) * t;

            Some([l, r])
        }
        _ => Some([l_l, l_r]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    /// 4 frames, interleaved: left = 0,1,2,3 and right = 10,11,12,13.
    fn track() -> TrackSamples {
        TrackSamples {
            samples: vec![0., 10., 1., 11., 2., 12., 3., 13.],
            rate: NonZeroU32::new(48_000).unwrap(),
        }
    }

    fn track_from(samples: Vec<f32>) -> TrackSamples {
        TrackSamples {
            samples,
            rate: NonZeroU32::new(48_000).unwrap(),
        }
    }

    #[track_caller]
    fn assert_frame(got: Option<[f32; 2]>, want: [f32; 2]) {
        let got = got.expect("expected Some(frame), got None");
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-5, "got {got:?}, want {want:?}");
        }
    }

    // ---- exact frames ----

    #[test]
    fn first_frame() {
        assert_frame(get_frame(&track(), 0.0), [0., 10.]);
    }

    #[test]
    fn middle_frames_keep_channels_in_order() {
        assert_frame(get_frame(&track(), 1.0), [1., 11.]);
        assert_frame(get_frame(&track(), 2.0), [2., 12.]);
    }

    #[test]
    fn last_frame() {
        assert_frame(get_frame(&track(), 3.0), [3., 13.]);
    }

    // ---- interpolation ----

    #[test]
    fn midpoint_between_first_two_frames() {
        assert_frame(get_frame(&track(), 0.5), [0.5, 10.5]);
    }

    #[test]
    fn quarter_point() {
        assert_frame(get_frame(&track(), 1.25), [1.25, 11.25]);
    }

    #[test]
    fn just_before_a_whole_frame_is_almost_that_frame() {
        assert_frame(get_frame(&track(), 1.999), [1.999, 11.999]);
    }

    // ---- end of track ----

    #[test]
    fn between_last_frame_and_the_end_holds_the_last_frame() {
        // the upper neighbour (frame 4) doesn't exist
        assert_frame(get_frame(&track(), 3.5), [3., 13.]);
    }

    #[test]
    fn one_past_the_last_frame_is_none() {
        assert_eq!(get_frame(&track(), 4.0), None);
    }

    #[test]
    fn far_past_the_end_is_none() {
        assert_eq!(get_frame(&track(), 1_000_000.0), None);
    }

    // ---- start of track (reverse playback running off the front) ----

    #[test]
    fn slightly_negative_is_none() {
        assert_eq!(get_frame(&track(), -0.1), None);
    }

    #[test]
    fn very_negative_is_none() {
        assert_eq!(get_frame(&track(), -1_000.0), None);
    }

    // ---- hostile floats: none of these may panic ----

    #[test]
    fn nan_is_none() {
        assert_eq!(get_frame(&track(), f64::NAN), None);
    }

    #[test]
    fn infinities_are_none() {
        assert_eq!(get_frame(&track(), f64::INFINITY), None);
        assert_eq!(get_frame(&track(), f64::NEG_INFINITY), None);
    }

    #[test]
    fn absurdly_large_is_none() {
        assert_eq!(get_frame(&track(), 1e30), None);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn value_that_overflows_the_index_multiplication_is_none() {
        // fits in a usize, but * CHANNELS overflows: this exercises checked_mul
        assert_eq!(get_frame(&track(), 1e19), None);
    }

    // ---- degenerate tracks ----

    #[test]
    fn empty_track_is_always_none() {
        let t = track_from(vec![]);
        assert_eq!(get_frame(&t, 0.0), None);
        assert_eq!(get_frame(&t, 0.5), None);
    }

    #[test]
    fn single_frame_track() {
        let t = track_from(vec![0.25, -0.25]);
        assert_frame(get_frame(&t, 0.0), [0.25, -0.25]);
        assert_frame(get_frame(&t, 0.5), [0.25, -0.25]); // holds the only frame
        assert_eq!(get_frame(&t, 1.0), None);
    }
}
