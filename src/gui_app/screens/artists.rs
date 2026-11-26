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
    library::{Artist, ArtistId, ManagerHandle},
};

pub struct ArtistsView {
    covers: CoverCache,
    artists: LoadState<Vec<Artist>>,
    manager: ManagerHandle,
    scroll: UniformListScrollHandle,
}

pub enum ArtistsEvent {
    Open(ArtistId),
}

impl EventEmitter<ArtistsEvent> for ArtistsView {}

impl HasCovers for ArtistsView {
    fn covers(&mut self) -> &mut CoverCache {
        &mut self.covers
    }
}

impl ArtistsView {
    pub(crate) fn new(cx: &mut Context<'_, ArtistsView>, manager: ManagerHandle) -> Self {
        Self::load(cx, manager.clone());

        Self {
            covers: CoverCache::default(),
            artists: LoadState::default(),
            manager,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn load(cx: &mut Context<Self>, manager: ManagerHandle) {
        spawn_load_on_library(
            cx,
            async move { manager.artists().await },
            |this, load_state| {
                this.artists = load_state;
            },
        );
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
                                ensure_cover(&mut this.covers, this.manager.clone(), cx, id);
                            }

                            let cover = cover_cell(
                                cover_id.and_then(|id| this.covers.get(&id)),
                                px(40.),
                                theme_muted,
                            );

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
            LoadState::Loaded(vec) => self.render_artists(cx, vec).into_any_element(),
            LoadState::Failed(s) => placeholder(s),
            LoadState::Loading => placeholder("Loading..."),
        }
    }
}
