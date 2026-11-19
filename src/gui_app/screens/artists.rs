use gpui_kit::{div, prelude::*};

use crate::gui_app::route::Route;

pub struct ArtistsView {}

impl ArtistsView {
    pub(crate) fn new(_cx: &mut Context<'_, ArtistsView>) -> Self {
        Self {}
    }
}

impl Render for ArtistsView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div().child(Route::Artists.title()).w_full()
    }
}
