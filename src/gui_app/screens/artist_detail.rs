use std::ops::Range;

use gpui_kit::{
    EventEmitter, UniformListScrollHandle,
    base::{h_flex, v_flex},
    component::ActiveTheme,
    prelude::*,
    px, uniform_list,
};

use crate::{
    gui_app::{
        cover::CoverCache,
        screens::{
            HasCovers, LoadState, cover_cell, ensure_cover, placeholder, spawn_load_on_library,
        },
    },
    library::{AlbumId, ArtistDetail, ArtistId, ManagerHandle},
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
        match &self.detail {
            LoadState::Loaded(detail) => self.view_detail(cx, detail).into_any_element(),
            LoadState::Failed(s) => placeholder(s),
            LoadState::Loading => placeholder("Loading..."),
        }
    }
}
