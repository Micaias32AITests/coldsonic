use std::ops::Range;

use gpui_kit::{
    EventEmitter, UniformListScrollHandle,
    base::{Button, StyledExt, h_flex, v_flex},
    component::ActiveTheme,
    div,
    prelude::*,
    px, uniform_list,
};

use crate::{
    engine::EngineHandle,
    gui_app::{
        cover::CoverCache,
        screens::{
            HasCovers, LoadState, cover_cell, ensure_cover, placeholder, spawn_load_on_library,
        },
    },
    library::{AlbumId, ArtistDetail, ArtistId, ManagerHandle},
    runtime::RT,
};

pub enum ArtistDetailEvent {
    Open(AlbumId),
}

pub struct ArtistDetailView {
    detail: LoadState<ArtistDetail>,
    covers: CoverCache,
    manager: ManagerHandle,
    scroll: UniformListScrollHandle,
}

impl EventEmitter<ArtistDetailEvent> for ArtistDetailView {}

impl HasCovers for ArtistDetailView {
    fn covers(&mut self) -> &mut CoverCache {
        &mut self.covers
    }
}

impl ArtistDetailView {
    pub fn new(cx: &mut Context<Self>, manager: ManagerHandle, artist_id: ArtistId) -> Self {
        let this = Self {
            detail: LoadState::Loading,
            covers: CoverCache::default(),
            manager: manager.clone(),
            scroll: UniformListScrollHandle::new(),
        };

        Self::load(cx, manager, artist_id);

        this
    }

    fn load(cx: &mut Context<Self>, manager: ManagerHandle, artist_id: ArtistId) {
        spawn_load_on_library(
            cx,
            async move { manager.artist(&artist_id).await },
            |this, load_state| {
                this.detail = load_state;
            },
        );
    }

    fn header(&self, cx: &mut Context<'_, Self>, detail: &ArtistDetail) -> impl IntoElement {
        let manager = self.manager.clone();
        let artist_id = detail.artist.id.clone();

        h_flex()
            .w_full()
            .gap_4()
            .child(cover_cell(
                detail
                    .artist
                    .cover
                    .as_ref()
                    .and_then(|c| self.covers.get(c)),
                px(256.),
                cx.theme().muted,
            ))
            .child(
                v_flex()
                    .child(div().text_size(px(24.)).child(detail.artist.name.clone()))
                    .child(format!("{n} albums", n = detail.artist.album_count))
                    .child(div().h_8())
                    .child(
                        h_flex().child(
                            Button::new("artist_detail.shuffle")
                                .child("Play Shuffled")
                                .on_click(move |_event, _window, cx| {
                                    let manager = manager.clone();
                                    let artist_id = artist_id.clone();

                                    let (tx, rx) = tokio::sync::oneshot::channel();
                                    RT::get(cx).spawn(async move {
                                        let _ = tx.send(manager.artist_tracks(artist_id).await);
                                    });

                                    cx.spawn(async move |cx| {
                                        let Ok(Ok(tracks)) = rx.await else {
                                            return;
                                        };

                                        cx.update(|cx| {
                                            EngineHandle::get(cx).update(cx, |engine, cx| {
                                                engine.play_queue_shuffled(cx, tracks);
                                            });
                                        });
                                    })
                                    .detach();
                                }),
                        ),
                    ),
            )
    }

    fn view_detail(&self, cx: &mut Context<Self>, detail: &ArtistDetail) -> impl IntoElement {
        v_flex().flex_1().h_full().min_h_0().child(
            uniform_list(
                "main.artist.albums",
                detail.albums.len(),
                cx.processor(|this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| {
                            let LoadState::Loaded(ref detail) = this.detail else {
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
                                ensure_cover(&mut this.covers, this.manager.clone(), cx, id);
                            }

                            let cover = cover_cell(
                                cover_id.and_then(|id| this.covers.get(&id)),
                                px(40.),
                                theme_muted,
                            );

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
        let detail = match &self.detail {
            LoadState::Loaded(detail) => detail,
            LoadState::Failed(s) => return placeholder(s),
            LoadState::Loading => return placeholder("Loading..."),
        };

        if let Some(ref cover_id) = detail.artist.cover {
            let manager = self.manager.clone();
            ensure_cover(&mut self.covers, manager, cx, cover_id);
        }

        v_flex()
            .margins(px(5.))
            .flex_1()
            .h_full()
            .min_h_0()
            .child(self.header(cx, detail))
            .child(self.view_detail(cx, detail))
            .into_any_element()
    }
}
