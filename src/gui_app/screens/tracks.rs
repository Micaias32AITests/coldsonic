use gpui_kit::{div, prelude::*};

use crate::gui_app::route::Route;

pub struct TracksView {}

impl TracksView {
    pub(crate) fn new(_cx: &mut Context<'_, TracksView>) -> Self {
        Self {}
    }
}

impl Render for TracksView {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div().child(Route::Tracks.title()).w_full()
    }
}
