use std::ops::Range;

use gpui_kit::{
    EventEmitter, ObjectFit, UniformListScrollHandle,
    base::{h_flex, v_flex},
    component::ActiveTheme,
    div, img,
    prelude::*,
    px, uniform_list,
};

use crate::{
    gui_app::{
        cover::{Cover, CoverCache, SMALL_COVER_PX, decode},
        screens::placeholder,
    },
    library::{AlbumId, ArtistDetail, ArtistId, CoverId, ManagerHandle},
};

pub enum ArtistDetailEvent {
    Open(AlbumId),
}

pub struct ArtistDetailView {
    detail: Option<ArtistDetail>,
    covers: CoverCache,
    manager: ManagerHandle,
    runtime: tokio::runtime::Handle,
    scroll: UniformListScrollHandle,
}

impl EventEmitter<ArtistDetailEvent> for ArtistDetailView {}

impl ArtistDetailView {
    pub fn new(
        cx: &mut Context<Self>,
        manager: ManagerHandle,
        runtime: tokio::runtime::Handle,
        artist_id: ArtistId,
    ) -> Self {
        let this = Self {
            detail: None,
            covers: CoverCache::default(),
            manager: manager.clone(),
            runtime: runtime.clone(),
            scroll: UniformListScrollHandle::new(),
        };

        cx.spawn(async move |this, cx| {
            let detail = runtime
                .block_on(manager.artist(&artist_id))
                .expect("load artist detail");

            this.update(cx, |this, cx| {
                this.detail = Some(detail);
                cx.notify();
            })
            .ok();
        })
        .detach();

        this
    }

    fn ensure_cover(&mut self, id: &CoverId, cx: &mut Context<Self>) {
        if !self.covers.start_fetch(id) {
            return;
        }
        let fetch_id = id.clone();

        let manager = self.manager.clone();
        let rt = self.runtime.clone();
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

    fn view_detail(&self, cx: &mut Context<Self>, detail: &ArtistDetail) -> impl IntoElement {
        v_flex().flex_1().h_full().min_h_0().child(
            uniform_list(
                "main.artist.albums",
                detail.albums.len(),
                cx.processor(|this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| {
                            let Some(ref detail) = this.detail else {
                                return "how??????".into_any_element();
                            };

                            let theme = cx.theme();
                            let theme_muted = theme.muted;
                            let theme_border = theme.border;

                            let album = &detail.albums[i];
                            let name = album.name.clone();
                            let album_id = album.id.clone();
                            let cover_id = album.cover.clone();

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
                                .id(("route.album_view.album", i))
                                .h(px(56.))
                                .w_full()
                                .px_4()
                                .gap_3()
                                .border_b_1()
                                .border_color(theme_border)
                                .child(cover)
                                .child(name)
                                .on_click(cx.listener(move |_this, _, _, cx| {
                                    cx.emit(ArtistDetailEvent::Open(album_id.clone()))
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

impl Render for ArtistDetailView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match &self.detail {
            Some(detail) => self.view_detail(cx, detail).into_any_element(),
            None => placeholder("Nothing to see here..."),
        }
    }
}
