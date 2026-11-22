pub mod cover;
pub mod route;
pub mod screens;

use std::time::Duration;

use gpui_kit::{
    AnyElement, Entity, Subscription, WindowOptions, application, assets,
    base::{h_flex, v_flex},
    component::{ActiveTheme, Theme, ThemeRegistry, button::Button},
    div, init, open_window,
    prelude::*,
    px,
};

use crate::gui_app::{
    route::Route,
    screens::{
        album_detail::{AlbumDetailEvent, AlbumDetailView},
        albums::AlbumsEvent,
        artists::{ArtistsEvent, ArtistsView},
        tracks::{TracksEvent, TracksView},
    },
};
use crate::gui_app::{
    route::RouteView,
    screens::{
        albums::AlbumsView,
        artist_detail::{ArtistDetailEvent, ArtistDetailView},
    },
};
use crate::{config, library::Manager};

struct Coldsonic {
    routes: Vec<StackEntry>,
    manager: Manager,
    runtime: tokio::runtime::Runtime,
}

impl Coldsonic {
    fn new(cx: &mut Context<Self>, rt: tokio::runtime::Runtime, manager: Manager) -> Self {
        let mut this = Self {
            routes: Vec::new(),
            manager,
            runtime: rt,
        };

        this.open_root_route(cx, Route::Albums);

        this
    }

    fn show_route(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        match self.routes.last() {
            Some(route) => route.view.clone().into_any_element(),
            None => div()
                .w_full()
                .h_full()
                .content_center()
                .child("Nothing to see here...")
                .into_any_element(),
        }
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
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_root_route(cx, route.clone());
            }))
    }

    fn open_root_route(&mut self, cx: &mut Context<Self>, route: Route) {
        self.routes.clear();

        let (view, subscriptions) = self.make_route(cx, route);
        self.routes.push(StackEntry {
            view,
            subscriptions,
        });
        cx.notify();
    }

    fn make_route(
        &mut self,
        cx: &mut Context<Self>,
        route: Route,
    ) -> (Entity<RouteView>, Vec<Subscription>) {
        let handle = self.manager.handle();
        let rt = self.runtime.handle().clone();

        match route {
            Route::Albums => {
                let albums = cx.new(|cx| AlbumsView::new(cx, handle, rt));
                let sub = cx.subscribe(&albums, Self::handle_albums_event);
                (cx.new(|_| RouteView::Albums(albums)), vec![sub])
            }
            Route::Artists => {
                let artists = cx.new(|cx| ArtistsView::new(cx, handle, rt));
                let sub = cx.subscribe(&artists, Self::handle_artists_event);
                (cx.new(|_cx| RouteView::Artists(artists)), vec![sub])
            }
            Route::Tracks => {
                let tracks = cx.new(|cx| TracksView::new(cx, handle, rt));
                let sub = cx.subscribe(&tracks, Self::handle_tracks_event);
                (cx.new(|_cx| RouteView::Tracks(tracks)), vec![sub])
            }
            Route::ArtistDetail(id) => {
                let art_detail = cx.new(|cx| ArtistDetailView::new(cx, handle, rt, id));
                let sub = cx.subscribe(&art_detail, Self::handle_artist_detail_event);
                (cx.new(|_cx| RouteView::ArtistDetail(art_detail)), vec![sub])
            }
            Route::AlbumDetail(id) => {
                let alb_detail = cx.new(|cx| AlbumDetailView::new(cx, handle, rt, id));
                let sub = cx.subscribe(&alb_detail, Self::handle_album_detail_event);
                (cx.new(|_cx| RouteView::AlbumDetail(alb_detail)), vec![sub])
            }
        }
    }

    fn handle_albums_event(
        &mut self,
        _albums: Entity<AlbumsView>,
        event: &AlbumsEvent,
        cx: &mut Context<'_, Coldsonic>,
    ) {
        match event {
            AlbumsEvent::Open(id) => {
                self.push_route(cx, Route::AlbumDetail(id.clone()));
            }
        }
    }

    fn handle_artists_event(
        &mut self,
        _artists: Entity<ArtistsView>,
        event: &ArtistsEvent,
        cx: &mut Context<'_, Coldsonic>,
    ) {
        match event {
            ArtistsEvent::Open(id) => {
                self.push_route(cx, Route::ArtistDetail(id.clone()));
            }
        }
    }

    fn handle_tracks_event(
        &mut self,
        _tracks: Entity<TracksView>,
        event: &TracksEvent,
        _cx: &mut Context<'_, Coldsonic>,
    ) {
        match event {
            TracksEvent::Open(id) => {
                tracing::debug!(track = %id.as_str(), "track clicked")
            }
        }
    }

    fn handle_artist_detail_event(
        &mut self,
        _albums: Entity<ArtistDetailView>,
        event: &ArtistDetailEvent,
        cx: &mut Context<'_, Coldsonic>,
    ) {
        match event {
            ArtistDetailEvent::Open(id) => {
                self.push_route(cx, Route::AlbumDetail(id.clone()));
            }
        }
    }

    fn handle_album_detail_event(
        &mut self,
        _albums: Entity<AlbumDetailView>,
        event: &AlbumDetailEvent,
        _cx: &mut Context<'_, Coldsonic>,
    ) {
        match event {
            AlbumDetailEvent::Open(id) => {
                tracing::debug!(track = %id.as_str(), "album's track clicked")
            }
            AlbumDetailEvent::PlayAlbum(id) => {
                tracing::debug!(album = %id.as_str(), "play album clicked")
            }
        }
    }

    #[expect(unused)]
    fn pop_route(&mut self, cx: &mut Context<Self>) {
        if !self.routes.is_empty() {
            _ = self.routes.pop();
            cx.notify();
        }
    }

    fn push_route(&mut self, cx: &mut Context<Self>, route: Route) {
        let (view, subscriptions) = self.make_route(cx, route);
        self.routes.push(StackEntry {
            view,
            subscriptions,
        });
        cx.notify();
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

struct StackEntry {
    view: Entity<RouteView>,
    #[expect(unused)]
    subscriptions: Vec<Subscription>,
}

pub fn run() {
    let libraries = config::load_libraries().expect("read libraries.toml");
    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
    let manager = runtime
        .block_on(Manager::open(&libraries))
        .expect("open library");

    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        let theme = ThemeRegistry::global(cx).default_dark_theme().clone();
        Theme::update(cx, |current| current.apply_config(&theme));
        open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| Coldsonic::new(cx, runtime, manager))
        })
        .expect("failed to open_window");
    })
}

pub fn format_duration(d: Duration) -> String {
    let h = if d.as_secs() / 60 / 60 > 0 {
        Some(d.as_secs() / 60 / 60)
    } else {
        None
    };

    let m = if h.is_some() {
        Some(d.as_secs() / 60 % 60)
    } else if d.as_secs() / 60 > 0 {
        Some(d.as_secs() / 60)
    } else {
        None
    };

    let s = if m.is_some() {
        d.as_secs() % 60
    } else {
        d.as_secs()
    };

    match (h, m) {
        (Some(h), Some(m)) => format!("{h:02}:{m:02}:{s:02} long"),
        (None, Some(m)) => format!("{m:02}:{s:02} long"),
        (None, None) => format!("{s:02} seconds long"),
        _ => "-1 seconds long".to_string(),
    }
}
