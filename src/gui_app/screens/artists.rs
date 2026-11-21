use std::ops::Range;

use gpui_kit::{
    EventEmitter, ObjectFit, SharedString, UniformListScrollHandle,
    base::{h_flex, v_flex},
    component::ActiveTheme,
    div, img,
    prelude::*,
    px, uniform_list,
};

use crate::{
    gui_app::cover::{Cover, CoverCache, SMALL_COVER_PX, decode},
    library::{Artist, ArtistId, CoverId, ManagerHandle},
};

pub struct ArtistsView {
    covers: CoverCache,
    artists: Artists,
    rt: tokio::runtime::Handle,
    manager: ManagerHandle,
    scroll: UniformListScrollHandle,
}

pub enum ArtistsEvent {
    Open(ArtistId),
}

impl EventEmitter<ArtistsEvent> for ArtistsView {}

pub enum Artists {
    Loading,
    Loaded(Vec<Artist>),
    Failed(SharedString),
}

impl ArtistsView {
    pub(crate) fn new(
        cx: &mut Context<'_, ArtistsView>,
        manager: ManagerHandle,
        rt: tokio::runtime::Handle,
    ) -> Self {
        let r = rt.clone();
        let m = manager.clone();
        cx.spawn(async move |this, cx| {
            let artists = r.clone().block_on(m.clone().artists());

            this.update(cx, |this, cx| {
                this.artists = match artists {
                    Ok(vec) => Artists::Loaded(vec),
                    Err(e) => Artists::Failed(e.to_string().into()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            covers: CoverCache::default(),
            artists: Artists::Loading,
            rt,
            manager,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn ensure_cover(&mut self, id: &CoverId, cx: &mut Context<Self>) {
        if !self.covers.start_fetch(id) {
            return;
        }
        let fetch_id = id.clone();

        let manager = self.manager.clone();
        let rt = self.rt.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();

        rt.spawn(async move {
            let _ = tx.send(manager.cover(&fetch_id, SMALL_COVER_PX).await);
        });

        let cover_id = id.clone();
        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else { return };

            let cover = match result {
                Ok(bytes) => decode(&bytes),
                Err(err) => {
                    tracing::debug!(%err, "cover fetch failed");
                    Cover::Failed
                }
            };

            this.update(cx, |this, cx| {
                if this.covers.end_fetch(cover_id, cover) {
                    cx.notify()
                }
            })
            .ok();
        })
        .detach();
    }

    fn render_artists(&self, cx: &mut Context<Self>, artists: &[Artist]) -> impl IntoElement {
        let a = artists.to_vec();
        v_flex().flex_1().h_full().min_h_0().child(
            uniform_list(
                "main.artists",
                artists.len(),
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| {
                            let theme = cx.theme();
                            let theme_muted = theme.muted;
                            let theme_border = theme.border;

                            let artist = &a[i];
                            let name = artist.name.clone();
                            let artist_id = artist.id.clone();
                            let cover_id = artist.cover.clone();

                            if let Some(id) = &cover_id {
                                this.ensure_cover(id, cx);
                            }

                            let placeholder_cover = move || {
                                div()
                                    .size(px(40.))
                                    .rounded_md()
                                    .bg(theme_muted)
                                    .into_any_element()
                            };
                            let cover = match cover_id.and_then(|id| this.covers.get(&id)) {
                                Some(Cover::Loaded(src)) => img(src.clone())
                                    .size(px(40.))
                                    .rounded_md()
                                    .object_fit(ObjectFit::Cover)
                                    .with_fallback(placeholder_cover)
                                    .into_any_element(),
                                _ => placeholder_cover(),
                            };

                            h_flex()
                                .id(("main.artists.artist", i))
                                .h(px(56.))
                                .w_full()
                                .px_4()
                                .gap_3()
                                .border_b_1()
                                .border_color(theme_border)
                                .child(cover)
                                .child(name)
                                .on_click(cx.listener(move |_this, _, _, cx| {
                                    cx.emit(ArtistsEvent::Open(artist_id.clone()))
                                }))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .h_full(),
        )
    }
}

impl Render for ArtistsView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match &self.artists {
            Artists::Loaded(vec) => self.render_artists(cx, vec).into_any_element(),
            Artists::Failed(s) => format!("failed to load artists: {s}").into_any_element(),
            Artists::Loading => "Loading...".into_any_element(),
        }
    }
}
