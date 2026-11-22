use std::ops::Range;

use gpui_kit::{
    AnyElement, EventEmitter, UniformListScrollHandle,
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
    library::{Album, AlbumId, ManagerHandle},
};

pub enum AlbumsEvent {
    Open(AlbumId),
}

pub struct AlbumsView {
    albums: LoadState<Vec<Album>>,
    covers: CoverCache,
    manager: ManagerHandle,
    rt: tokio::runtime::Handle,
    scroll: UniformListScrollHandle,
}

impl EventEmitter<AlbumsEvent> for AlbumsView {}

impl HasCovers for AlbumsView {
    fn covers(&mut self) -> &mut CoverCache {
        &mut self.covers
    }
}

impl AlbumsView {
    pub fn new(cx: &mut Context<Self>, manager: ManagerHandle, rt: tokio::runtime::Handle) -> Self {
        let this = Self {
            albums: LoadState::default(),
            covers: CoverCache::default(),
            manager: manager.clone(),
            rt: rt.clone(),
            scroll: UniformListScrollHandle::new(),
        };

        Self::load(cx, manager, rt);

        this
    }

    fn load(cx: &mut Context<Self>, manager: ManagerHandle, rt: tokio::runtime::Handle) {
        spawn_load_on_library(
            cx,
            rt,
            async move { manager.albums().await },
            |this, load_state| this.albums = load_state,
        );
    }

    fn show_albums(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(s) = self.albums.state_message() {
            return placeholder(s);
        }
        if self.albums.items().is_empty() {
            return placeholder("No items.");
        }

        v_flex()
            .flex_1()
            .h_full()
            .min_h_0()
            .child(
                uniform_list(
                    "main.albums",
                    self.albums.items().len(),
                    cx.processor(|this, range: Range<usize>, _window, cx| {
                        range
                            .map(|i| {
                                let theme = cx.theme();
                                let theme_muted = theme.muted;
                                let theme_border = theme.border;

                                let album = &this.albums.items()[i];
                                let name = album.name.clone();
                                let album_id = album.id.clone();
                                let cover_id = album.cover.clone();

                                if let Some(id) = &cover_id {
                                    ensure_cover(
                                        &mut this.covers,
                                        this.manager.clone(),
                                        &this.rt,
                                        cx,
                                        id,
                                    );
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
                                        cx.emit(AlbumsEvent::Open(album_id.clone()))
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

impl Render for AlbumsView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.show_albums(cx)
    }
}
