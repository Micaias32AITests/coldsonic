pub mod cover;
pub mod route;
pub mod screens;

use gpui_kit::{
    AnyElement, Entity, Subscription, WindowOptions, application, assets,
    base::{h_flex, v_flex},
    component::{ActiveTheme, button::Button},
    div, init, open_window,
    prelude::*,
    px,
};

use crate::gui_app::{
    route::Route,
    screens::{albums::AlbumsEvent, artists::ArtistsView, tracks::TracksView},
};
use crate::gui_app::{route::RouteView, screens::albums::AlbumsView};
use crate::{config, library::Manager};

struct Coldsonic {
    routes: Vec<StackEntry>,
    manager: Manager,
    /// should go away soon
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
                this.open_root_route(cx, route);
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
            Route::Artists => (
                cx.new(|cx| RouteView::Artists(cx.new(ArtistsView::new))),
                Vec::new(),
            ),
            Route::Tracks => (
                cx.new(|cx| RouteView::Tracks(cx.new(TracksView::new))),
                Vec::new(),
            ),
        }
    }

    fn handle_albums_event(
        &mut self,
        _albums: Entity<AlbumsView>,
        event: &AlbumsEvent,
        _cx: &mut Context<'_, Coldsonic>,
    ) {
        match event {
            AlbumsEvent::Open(id) => {
                tracing::debug!(album = %id.as_str(), "album clicked")
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

    #[expect(unused)]
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
        open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| Coldsonic::new(cx, runtime, manager))
        })
        .expect("failed to open_window");
    })
}
