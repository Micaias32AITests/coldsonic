use gpui_kit::prelude::*;

use crate::library::ManagerHandle;

pub mod now_playing;
pub mod queue;

pub struct RightBar {
    #[expect(unused)]
    manager: ManagerHandle,
}

impl RightBar {
    pub fn new(_cx: &mut Context<Self>, manager: ManagerHandle) -> Self {
        Self { manager }
    }
}

impl Render for RightBar {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        "Sidebar"
    }
}
