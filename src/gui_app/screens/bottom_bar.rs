use std::time::Duration;

use gpui_kit::{
    Entity, ImageSource, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme,
        button::{Button, ButtonVariants},
        slider::{Slider, SliderEvent, SliderState},
    },
    div,
    prelude::*,
    px,
};

use crate::{
    engine::{Engine, EngineEvent, EngineHandle, NowPlaying, PlaybackState, Track},
    gui_app::{
        cover::{self, SMALL_COVER_PX, decode_image},
        screens::cover_cell,
    },
    library::ManagerHandle,
    runtime::RT,
};

/// Logical cover size in the bar.
const COVER_DISPLAY: f32 = 48.0;

/// Height of the bar, matching the iced layout (48 px cover + 8 px padding).
const BAR_HEIGHT: f32 = 64.0;

pub struct BottomBar {
    engine: Entity<Engine>,
    _engine_sub: Subscription,
    seek: Entity<SliderState>,
    _seek_sub: Subscription,
    volume: Entity<SliderState>,
    _volume_sub: Subscription,
    current_cover: Cover,
    manager: ManagerHandle,
    now_playing: Option<NowPlaying>,
    /// Draft seek position (0.0–1.0) while the user drags the progress slider.
    seeking: Option<f32>,
    /// Volume restored by the mute toggle.
    last_volume: f32,
}

#[derive(Default, Clone)]
pub enum Cover {
    #[default]
    None,
    Loading(Track),
    Loaded(ImageSource, Track),
    Failed(Track),
}

impl From<Cover> for Option<cover::Cover> {
    fn from(value: Cover) -> Self {
        match value {
            Cover::None => None,
            Cover::Loading(_) => Some(cover::Cover::Loading),
            Cover::Loaded(image, _) => Some(cover::Cover::Loaded(image)),
            Cover::Failed(_) => Some(cover::Cover::Failed),
        }
    }
}

impl BottomBar {
    pub fn new(cx: &mut Context<Self>, manager: ManagerHandle) -> Self {
        let engine = EngineHandle::get(cx);

        let sub = cx.subscribe(&engine, |this, engine, event: &EngineEvent, cx| {
            match event {
                EngineEvent::PlaybackStarted(_) | EngineEvent::Stopped | EngineEvent::SeekDone => {
                    this.seeking = None;
                }
                _ => {}
            }
            this.now_playing = engine.read(cx).now_playing();
            this.load_cover(cx);
            cx.notify();
        });

        let seek = cx.new(|_| SliderState::new().min(0.0).max(1.0).step(0.001));
        let seek_sub = cx.subscribe(&seek, |this, _, event: &SliderEvent, cx| match event {
            SliderEvent::Change(value) => {
                this.seeking = Some(value.end());
                cx.notify();
            }
            SliderEvent::Release(value) => {
                let fraction = value.end().clamp(0.0, 1.0);
                this.seeking = None;
                if let Some(now_playing) = &this.now_playing {
                    let position = now_playing.duration.mul_f32(fraction);
                    this.engine
                        .update(cx, |engine, cx| engine.seek(cx, position));
                }
                cx.notify();
            }
        });

        let volume = cx.new(|_| SliderState::new().min(0.0).max(1.0).step(0.01));
        let volume_sub = cx.subscribe(&volume, |this, _, event: &SliderEvent, cx| {
            let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
            let volume = value.end().clamp(0.0, 1.0);
            if volume > 0.0 {
                this.last_volume = volume;
            }
            this.engine
                .update(cx, |engine, cx| engine.set_volume(cx, volume));
        });

        let now_playing = engine.read(cx).now_playing();
        let last_volume = engine.read(cx).volume();

        let mut this = Self {
            engine,
            _engine_sub: sub,
            seek,
            _seek_sub: seek_sub,
            volume,
            _volume_sub: volume_sub,
            current_cover: Cover::None,
            manager,
            now_playing,
            seeking: None,
            last_volume,
        };
        this.load_cover(cx);
        this
    }

    /// Fetch the current track's cover only when the track actually changed.
    /// `Loading/Loaded/Failed` all carry the track, so a pending or failed
    /// fetch does not re-spawn on every tick.
    fn load_cover(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.now_playing.as_ref().map(|n| n.track.clone()) else {
            self.current_cover = Cover::None;
            return;
        };

        if let Cover::Loading(t) | Cover::Loaded(_, t) | Cover::Failed(t) = &self.current_cover
            && t.id == track.id
        {
            return;
        }

        let Some(cover_id) = track.cover.clone() else {
            self.current_cover = Cover::None;
            return;
        };

        self.current_cover = Cover::Loading(track.clone());

        let (tx, rx) = tokio::sync::oneshot::channel();
        let rt = RT::get(cx);
        let manager = self.manager.clone();

        rt.spawn(async move {
            let res = manager.cover(&cover_id, SMALL_COVER_PX).await;
            let _ = tx.send((track, res));
        });

        cx.spawn(async move |this, cx| {
            let Ok((track, res)) = rx.await else {
                return;
            };

            this.update(cx, |this, cx| {
                this.current_cover = match res {
                    Ok(bytes) => Cover::Loaded(decode_image(&bytes), track),
                    Err(_) => Cover::Failed(track),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keep the sliders in step with the engine unless the user is dragging the
    /// seek slider. Runs in `render` because `set_value` needs a `Window`.
    fn sync_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let position = match &self.now_playing {
            Some(now_playing) => {
                let total = now_playing.duration.as_secs_f32();

                if total > 0.0 {
                    (now_playing.position.as_secs_f32() / total).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        let volume = self.engine.read(cx).volume();

        if self.seeking.is_none() {
            self.seek.update(cx, |state, cx| {
                if (state.value().end() - position).abs() > 0.0005 {
                    state.set_value(position, window, cx);
                }
            });
        }

        self.volume.update(cx, |state, cx| {
            if (state.value().end() - volume).abs() > 0.0005 {
                state.set_value(volume, window, cx);
            }
        });
    }

    fn icon_button(
        &self,
        id: &'static str,
        icon: IconName,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Engine, &mut Context<Engine>) + 'static,
    ) -> Button {
        Button::new(id)
            .icon(icon)
            .ghost()
            .compact()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.engine.update(cx, |engine, cx| on_click(engine, cx));
            }))
    }
}

impl Render for BottomBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_sliders(window, cx);

        let (title, artist, position, duration, state) = match &self.now_playing {
            Some(now_playing) => (
                now_playing.track.title.clone(),
                now_playing.track.artist.clone().unwrap_or_default(),
                now_playing.position,
                now_playing.duration,
                Some(now_playing.state),
            ),
            None => (
                String::new(),
                String::new(),
                Duration::ZERO,
                Duration::ZERO,
                None,
            ),
        };

        let volume = self.engine.read(cx).volume();
        let muted = volume <= 0.0;
        let play_icon = match state {
            Some(PlaybackState::Paused) => IconName::Play,
            Some(_) => IconName::Pause,
            None => IconName::Play,
        };

        // Left: cover + title/artist.
        let left = h_flex()
            .flex_1()
            .flex_grow(1.0)
            .min_w_0()
            .max_w(px(400.))
            .gap_2()
            .child(cover_cell(
                Option::from(self.current_cover.clone()).as_ref(),
                px(COVER_DISPLAY),
                cx.theme().muted,
            ))
            .child(
                v_flex()
                    .min_w_0()
                    .child(div().truncate().text_sm().child(title))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(artist),
                    ),
            );

        // Center: transport + progress.
        let middle = v_flex()
            .flex_1()
            .flex_grow(2.0)
            .items_center()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        self.icon_button("bottom.prev", IconName::SkipBack, cx, |engine, cx| {
                            engine.prev(cx);
                        }),
                    )
                    .child(
                        self.icon_button("bottom.play", play_icon, cx, |engine, cx| {
                            engine.toggle(cx);
                        }),
                    )
                    .child(self.icon_button(
                        "bottom.next",
                        IconName::SkipForward,
                        cx,
                        |engine, cx| {
                            engine.next(cx);
                        },
                    ))
                    .child(
                        self.icon_button("bottom.stop", IconName::Square, cx, |engine, cx| {
                            engine.stop(cx);
                        }),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(div().text_xs().child(fmt_duration(position)))
                    .child(Slider::new(&self.seek).flex_1())
                    .child(div().text_xs().child(fmt_duration(duration))),
            );

        // Right: mute + volume.
        let right = h_flex()
            .flex_1()
            .flex_grow(1.0)
            .min_w_0()
            .max_w(px(400.))
            .gap_2()
            .justify_end()
            .child(
                Button::new("bottom.mute")
                    .icon(if muted {
                        IconName::VolumeX
                    } else {
                        IconName::Volume2
                    })
                    .ghost()
                    .compact()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let volume = this.engine.read(cx).volume();
                        let next = if volume <= 0.0 {
                            this.last_volume
                        } else {
                            this.last_volume = volume;
                            0.0
                        };
                        this.engine
                            .update(cx, |engine, cx| engine.set_volume(cx, next));
                    })),
            )
            .child(Slider::new(&self.volume).w(px(100.)));

        h_flex()
            .w_full()
            .justify_between()
            .h(px(BAR_HEIGHT))
            .px(px(8.))
            .gap_3()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().foreground)
            .child(left)
            .child(middle)
            .child(right)
    }
}

/// `mm:ss`, matching the iced bar.
fn fmt_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}
