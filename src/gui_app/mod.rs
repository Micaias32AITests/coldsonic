use std::{collections::HashMap, ops::Range};

use std::sync::Arc;

use gpui_kit::{
    AnyElement, Image, ImageFormat, ImageSource, ObjectFit, WindowOptions, application, assets,
    base::{h_flex, v_flex},
    component::{
        ActiveTheme,
        button::{Button, ButtonVariants},
    },
    div, img, init, open_window,
    prelude::*,
    px, uniform_list,
};

use crate::{
    config,
    library::{Album, CoverId, Manager},
};

const SMALL_COVER_PX: u32 = 96;

struct Coldsonic {
    route: Route,
    albums: Vec<Album>,
    manager: Manager,
    /// should go away soon
    runtime: tokio::runtime::Runtime,
    cover_cache: HashMap<CoverId, Cover>,
}

impl Coldsonic {
    fn new(rt: tokio::runtime::Runtime, manager: Manager, albums: Vec<Album>) -> Self {
        Self {
            route: Route::Albums,
            manager,
            albums,
            cover_cache: HashMap::new(),
            runtime: rt,
        }
    }

    fn show_route(&mut self, cx: &mut Context<Self>) -> AnyElement {
        match self.route {
            Route::Albums => self.show_albums(cx).into_any_element(),
            Route::Artists => self.show_artists(cx).into_any_element(),
            Route::Tracks => self.show_tracks(cx).into_any_element(),
        }
    }

    fn show_albums(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().flex_1().h_full().min_h_0().child(
            uniform_list(
                "main.albums",
                self.albums.len(),
                cx.processor(|this, range: Range<usize>, _window, cx| {
                    range
                        .map(|i| {
                            let theme = cx.theme();
                            let theme_muted = theme.muted;
                            let theme_border = theme.border;

                            let album = &this.albums[i];
                            let name = album.name.clone();
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
                            let cover = match cover_id.and_then(|id| this.cover_cache.get(&id)) {
                                Some(Cover::Loaded(src)) => img(src.clone())
                                    .size(px(40.))
                                    .rounded_md()
                                    .object_fit(ObjectFit::Cover)
                                    .with_fallback(placeholder_cover)
                                    .into_any_element(),
                                _ => placeholder_cover(),
                            };

                            h_flex()
                                .h(px(56.))
                                .w_full()
                                .px_4()
                                .gap_3()
                                .border_b_1()
                                .border_color(theme_border)
                                .child(cover)
                                .child(name)
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .h_full(),
        )
    }

    fn show_artists(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(Route::Artists.title()).w_full()
    }

    fn show_tracks(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(Route::Tracks.title()).w_full()
    }

    fn show_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(200.))
            .h_full()
            .bg(cx.theme().sidebar)
            .child(self.nav_button(cx, "sidebar.albums", "Albums", Route::Albums))
            .child(self.nav_button(cx, "sidebar.artists", "Artists", Route::Artists))
            .child(self.nav_button(cx, "sidebar.tracks", "Tracks", Route::Tracks))
    }

    fn nav_button(
        &self,
        cx: &mut Context<Self>,
        id: &'static str,
        label: &'static str,
        route: Route,
    ) -> Button {
        Button::new(id)
            .label(label)
            .when(self.route == route, |button| button.primary())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.route = route;
                cx.notify();
            }))
    }

    fn ensure_cover(&mut self, id: &CoverId, cx: &mut Context<Self>) {
        if self.cover_cache.contains_key(id) {
            return;
        }
        self.cover_cache.insert(id.clone(), Cover::Loading);

        let handle = self.manager.handle();
        let rt = &self.runtime;
        let fetch_id = id.clone();

        let (tx, rx) = tokio::sync::oneshot::channel();

        rt.spawn(async move {
            let _ = tx.send(handle.cover(&fetch_id, SMALL_COVER_PX).await);
        });

        let cover_id = id.clone();
        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else { return };

            this.update(cx, |this, cx| {
                if !matches!(this.cover_cache.get(&cover_id), Some(Cover::Loading)) {
                    return;
                }

                let cover = match result {
                    Ok(bytes) => Cover::Loaded(
                        Arc::new(Image::from_bytes(image_format(&bytes), bytes.to_vec())).into(),
                    ),
                    Err(_) => Cover::Failed,
                };

                this.cover_cache.insert(cover_id, cover);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for Coldsonic {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .size_full()
                    .child(self.show_sidebar(cx))
                    .child(self.show_route(cx))
                    .child(div().w(px(320.)).h_full().bg(cx.theme().sidebar)),
            )
    }
}

pub fn run() {
    let libraries = config::load_libraries().expect("read libraries.toml");
    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
    let manager = runtime
        .block_on(Manager::open(&libraries))
        .expect("open library");
    let albums = runtime
        .block_on(manager.handle().albums())
        .expect("load albums");

    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| Coldsonic::new(runtime, manager, albums))
        })
        .expect("failed to open_window");
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Albums,
    Artists,
    Tracks,
}

impl Route {
    fn title(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Tracks => "Tracks",
        }
    }
}

enum Cover {
    Loading,
    Loaded(ImageSource),
    Failed,
}

fn image_format(bytes: &[u8]) -> gpui_kit::ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        ImageFormat::Jpeg
    } else if bytes.starts_with(b"RIFF") {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}
