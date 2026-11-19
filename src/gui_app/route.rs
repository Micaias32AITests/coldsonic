use gpui_kit::{Entity, Render, prelude::*};

use crate::gui_app::screens::{albums::AlbumsView, artists::ArtistsView, tracks::TracksView};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Albums,
    Artists,
    Tracks,
}

impl Route {
    pub fn title(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Tracks => "Tracks",
        }
    }
}

pub enum RouteView {
    Albums(Entity<AlbumsView>),
    Artists(Entity<ArtistsView>),
    Tracks(Entity<TracksView>),
}

impl Render for RouteView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match self {
            Self::Albums(view) => view.clone().into_any_element(),
            Self::Artists(view) => view.clone().into_any_element(),
            Self::Tracks(view) => view.clone().into_any_element(),
        }
    }
}
