use gpui_kit::{Entity, Render, prelude::*};

use crate::{
    gui_app::screens::{
        album_detail::AlbumDetailView, albums::AlbumsView, artist_detail::ArtistDetailView,
        artists::ArtistsView, tracks::TracksView,
    },
    library::{AlbumId, ArtistId},
};

#[derive(Clone, PartialEq, Eq)]
pub enum Route {
    Albums,
    Artists,
    Tracks,
    ArtistDetail(ArtistId),
    AlbumDetail(AlbumId),
}

pub enum RouteView {
    Albums(Entity<AlbumsView>),
    AlbumDetail(Entity<AlbumDetailView>),
    Artists(Entity<ArtistsView>),
    ArtistDetail(Entity<ArtistDetailView>),
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
            Self::ArtistDetail(view) => view.clone().into_any_element(),
            Self::AlbumDetail(view) => view.clone().into_any_element(),
        }
    }
}
