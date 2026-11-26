use std::ops::Range;

use gpui_kit::{
    UniformListScrollHandle,
    base::{h_flex, v_flex},
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
    library::{ManagerHandle, Track},
};

pub struct TracksView {
    covers: CoverCache,
    tracks: LoadState<Vec<Track>>,
    manager: ManagerHandle,
    scroll: UniformListScrollHandle,
}

impl HasCovers for TracksView {
    fn covers(&mut self) -> &mut CoverCache {
        &mut self.covers
    }
}

impl TracksView {
    pub(crate) fn new(cx: &mut Context<'_, TracksView>, manager: ManagerHandle) -> Self {
        Self::load(cx, manager.clone());
        Self {
            covers: CoverCache::default(),
            tracks: LoadState::default(),
            manager,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn load(cx: &mut Context<Self>, manager: ManagerHandle) {
        spawn_load_on_library(
            cx,
            async move { manager.tracks().await },
            |this, load_state| this.tracks = load_state,
        );
    }

    fn render_tracks(&self, cx: &mut Context<Self>, tracks: &[Track]) -> impl IntoElement {
        v_flex().flex_1().h_full().min_h_0().child(
            uniform_list(
                "main.tracks",
                tracks.len(),
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| {
                            let Some(tracks) = this.tracks.loaded() else {
                                unreachable!("calling render tracks without tracks to render");
                            };
                            let theme = cx.theme();
                            let theme_muted = theme.muted;
                            let theme_muted_fg = theme.muted_foreground;
                            let theme_border = theme.border;

                            let track = tracks[i].clone();
                            let name = track.title.clone();
                            let artist = track
                                .artist
                                .clone()
                                .unwrap_or_else(|| "Unknown Artist".into());
                            let cover_id = track.cover.clone();

                            if let Some(id) = &cover_id {
                                ensure_cover(&mut this.covers, this.manager.clone(), cx, id);
                            }

                            let cover = cover_cell(
                                cover_id.and_then(|id| this.covers.get(&id)),
                                px(40.),
                                theme_muted,
                            );

                            h_flex()
                                .id(("main.tracks.track", i))
                                .h(px(56.))
                                .w_full()
                                .px_4()
                                .gap_3()
                                .border_b_1()
                                .border_color(theme_border)
                                .child(cover)
                                .child(
                                    v_flex()
                                        .child(name)
                                        .child(div().text_color(theme_muted_fg).child(artist)),
                                )
                                .on_click(cx.listener(move |_this, _, _, cx| {
                                    EngineHandle::get(cx).update(cx, |engine, cx| {
                                        engine.play_now(cx, track.clone());
                                    })
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

impl Render for TracksView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match &self.tracks {
            LoadState::Loaded(vec) => self.render_tracks(cx, vec).into_any_element(),
            LoadState::Failed(s) => placeholder(s),
            LoadState::Loading => placeholder("Loading..."),
        }
    }
}
