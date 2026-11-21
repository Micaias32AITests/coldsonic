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
    library::{AlbumDetail, AlbumId, CoverId, ManagerHandle, TrackId},
};

pub struct AlbumDetailView {
    covers: CoverCache,
    detail: Detail,
    rt: tokio::runtime::Handle,
    manager: ManagerHandle,
    scroll: UniformListScrollHandle,
}

pub enum AlbumDetailEvent {
    Open(TrackId),
    PlayAlbum(AlbumId),
}

impl EventEmitter<AlbumDetailEvent> for AlbumDetailView {}

pub enum Detail {
    Loading,
    Loaded(AlbumDetail),
    Failed(SharedString),
}

impl AlbumDetailView {
    pub(crate) fn new(
        cx: &mut Context<'_, AlbumDetailView>,
        manager: ManagerHandle,
        rt: tokio::runtime::Handle,
        album_id: AlbumId,
    ) -> Self {
        let r = rt.clone();
        let m = manager.clone();
        cx.spawn(async move |this, cx| {
            let detail = r.clone().block_on(m.clone().album(&album_id));

            this.update(cx, |this, cx| {
                this.detail = match detail {
                    Ok(detail) => Detail::Loaded(detail),
                    Err(e) => Detail::Failed(e.to_string().into()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            covers: CoverCache::default(),
            detail: Detail::Loading,
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

    fn render_detail(&self, cx: &mut Context<Self>, detail: &AlbumDetail) -> impl IntoElement {
        v_flex().flex_1().h_full().min_h_0().child(
            uniform_list(
                "main.tracks",
                detail.songs.len(),
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| {
                            let Detail::Loaded(ref detail) = this.detail else {
                                return "how??????".into_any_element();
                            };

                            let theme = cx.theme();
                            let theme_muted = theme.muted;
                            let theme_border = theme.border;

                            let track = &detail.songs[i];
                            let name = track.title.clone();
                            let track_id = track.id.clone();
                            let cover_id = track.cover.clone();

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
                                .id(("main.album.track", i))
                                .h(px(56.))
                                .w_full()
                                .px_4()
                                .gap_3()
                                .border_b_1()
                                .border_color(theme_border)
                                .child(cover)
                                .child(name)
                                .on_click(cx.listener(move |_this, _, _, cx| {
                                    cx.emit(AlbumDetailEvent::Open(track_id.clone()))
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

impl Render for AlbumDetailView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match &self.detail {
            Detail::Loaded(detail) => self.render_detail(cx, detail).into_any_element(),
            Detail::Failed(s) => format!("failed to load tracks: {s}").into_any_element(),
            Detail::Loading => "Loading...".into_any_element(),
        }
    }
}
