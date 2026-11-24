use crate::{
    engine::{Engine, EngineEvent, EngineHandle, NowPlaying, Track},
    gui_app::{
        cover::{self, SMALL_COVER_PX, decode_image},
        screens::cover_cell,
    },
    library::ManagerHandle,
    runtime::RT,
};

use gpui_kit::{
    Edges, Entity, ImageSource, Subscription,
    base::{StyledExt, h_flex},
    component::{ActiveTheme, slider::Slider},
    prelude::*,
    px,
};

pub struct BottomBar {
    engine: Entity<Engine>,
    _engine_sub: Subscription,
    current_cover: Cover,
    manager: ManagerHandle,
    now_playing: Option<NowPlaying>,
}

#[derive(Default, Clone)]
pub enum Cover {
    #[default]
    None,
    Loading,
    Loaded(ImageSource, Track),
    Failed,
}

impl From<Cover> for Option<cover::Cover> {
    fn from(value: Cover) -> Self {
        match value {
            Cover::None => None,
            Cover::Loading => Some(cover::Cover::Loading),
            Cover::Loaded(i, _t) => Some(cover::Cover::Loaded(i)),
            Cover::Failed => Some(cover::Cover::Failed),
        }
    }
}

impl BottomBar {
    pub fn new(cx: &mut Context<Self>, manager: ManagerHandle) -> Self {
        let engine = EngineHandle::get(cx);
        let sub = cx.subscribe(&engine, |this, engine, event, cx| match event {
            EngineEvent::UpdateNowPlaying => {
                this.now_playing = engine.read(cx).now_playing();
                this.load_cover(cx);
                cx.notify();
            }
            _ => {
                tracing::debug!("woah");
            }
        });

        let now_playing = engine.read(cx).now_playing();
        let mut this = Self {
            engine,
            _engine_sub: sub,
            current_cover: Cover::None,
            manager,
            now_playing,
        };
        this.load_cover(cx);
        this
    }

    fn load_cover(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.now_playing.as_ref().map(|n| n.track.clone()) else {
            return;
        };
        if matches!(&self.current_cover, Cover::Loaded(_, t) if t.id == track.id)
            || matches!(&self.current_cover, Cover::Failed)
        {
            return;
        }
        let Some(cover_id) = track.cover.clone() else {
            return;
        };

        self.current_cover = Cover::Loading;

        let (tx, rx) = tokio::sync::oneshot::channel();
        let rt = RT::get(cx);

        let manager = self.manager.clone();
        rt.spawn(async move {
            let _ = tx.send((track, manager.cover(&cover_id, SMALL_COVER_PX * 2).await));
        });

        cx.spawn(async move |this, cx| {
            let Ok((track, res)) = rx.await else {
                return;
            };

            this.update(cx, |this, cx| {
                let image = match res {
                    Ok(bytes) => decode_image(&bytes),
                    Err(_e) => {
                        this.current_cover = Cover::Failed;
                        cx.notify();
                        return;
                    }
                };
                this.current_cover = Cover::Loaded(image, track);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for BottomBar {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .margins(Edges {
                left: px(10.),
                right: px(10.),
                ..Default::default()
            })
            .bg(cx.theme().sidebar)
            .w_full()
            .h(px(100.))
            .child(cover_cell(
                Option::from(self.current_cover.clone()).as_ref(),
                px(80.),
                cx.theme().muted,
            ))
            .child(
                self.now_playing
                    .as_ref()
                    .map(|n| n.track.title.clone())
                    .unwrap_or_default(),
            )
    }
}
