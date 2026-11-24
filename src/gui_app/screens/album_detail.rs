use std::ops::Range;

use gpui_kit::{
    EventEmitter, UniformListScrollHandle,
    base::{StyledExt, h_flex, v_flex},
    component::{ActiveTheme, button::Button},
    div,
    prelude::*,
    px, uniform_list,
};

use crate::{
    engine::EngineHandle,
    gui_app::{
        cover::CoverCache,
        format_duration,
        screens::{
            HasCovers, LoadState, cover_cell, ensure_cover, placeholder, spawn_load_on_library,
        },
    },
    library::{AlbumDetail, AlbumId, ManagerHandle, TrackId},
};

pub struct AlbumDetailView {
    covers: CoverCache,
    detail: LoadState<AlbumDetail>,
    manager: ManagerHandle,
    scroll: UniformListScrollHandle,
}

pub enum AlbumDetailEvent {
    Open(TrackId),
    PlayAlbum(AlbumId),
}

impl EventEmitter<AlbumDetailEvent> for AlbumDetailView {}

impl HasCovers for AlbumDetailView {
    fn covers(&mut self) -> &mut CoverCache {
        &mut self.covers
    }
}

impl AlbumDetailView {
    pub fn new(
        cx: &mut Context<'_, AlbumDetailView>,
        manager: ManagerHandle,
        album_id: AlbumId,
    ) -> Self {
        Self::load(cx, manager.clone(), album_id);
        Self {
            covers: CoverCache::default(),
            detail: LoadState::default(),
            manager,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn load(cx: &mut Context<Self>, manager: ManagerHandle, album_id: AlbumId) {
        spawn_load_on_library(
            cx,
            async move { manager.album(&album_id).await },
            |this, load_state| this.detail = load_state,
        );
    }

    fn header(&self, cx: &mut Context<'_, Self>, detail: &AlbumDetail) -> impl IntoElement {
        let tracks = detail.songs.clone();
        h_flex()
            .w_full()
            .gap_4()
            .child(cover_cell(
                detail.album.cover.as_ref().and_then(|c| self.covers.get(c)),
                px(256.),
                cx.theme().muted,
            ))
            .child(
                v_flex()
                    .child(div().text_size(px(24.)).child(detail.album.name.clone()))
                    .child(
                        detail
                            .album
                            .artist
                            .clone()
                            .unwrap_or_else(|| "Unknown Artist".into()),
                    )
                    .child(detail.album.duration.map(format_duration).map_or_else(
                        || format!("{n} tracks", n = detail.album.song_count),
                        |s| format!("{s} | {n} tracks", n = detail.album.song_count),
                    ))
                    .child(div().h_8())
                    .child(h_flex().child(
                        Button::new("album_detail.play").child("Play").on_click(
                            move |_event, _window, cx| {
                                EngineHandle::get(cx).update(cx, |engine, cx| {
                                    engine.play_queue(cx, tracks.clone());
                                });
                            },
                        ),
                    )),
            )
    }
}

impl Render for AlbumDetailView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let detail = match &self.detail {
            LoadState::Loaded(detail) => detail,
            LoadState::Loading => return placeholder("Loading tracks..."),
            LoadState::Failed(s) => return placeholder(s),
        };

        if let Some(ref cover_id) = detail.album.cover {
            let manager = self.manager.clone();
            ensure_cover(&mut self.covers, manager, cx, cover_id);
        }
        v_flex()
            .margins(px(5.))
            .flex_1()
            .h_full()
            .min_h_0()
            .child(self.header(cx, detail))
            .child(
                uniform_list(
                    "main.tracks",
                    detail.songs.len(),
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        range
                            .map(|i| {
                                let manager = this.manager.clone();

                                let Some(detail) = this.detail.loaded() else {
                                    unreachable!("should be loaded");
                                };
                                let theme = cx.theme();
                                let theme_muted = theme.muted;
                                let theme_border = theme.border;

                                let track = &detail.songs[i];
                                let name = track.title.clone();
                                let track_id = track.id.clone();
                                let cover_id = track.cover.clone();

                                if let Some(id) = &cover_id {
                                    ensure_cover(this.covers(), manager, cx, id);
                                }

                                let cover = cover_cell(
                                    cover_id.and_then(|id| this.covers.get(&id)),
                                    px(40.),
                                    theme_muted,
                                );

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
            .into_any_element()
    }
}
