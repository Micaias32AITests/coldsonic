// TODO(ui_01 phase 4): implement `Screen` for `State` (init/update/view) and
// render the engine queue. Held by the right panel, not the navigation stack.
// Placeholder screen until then.

use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {}

impl State {
    pub fn new() -> Self {
        Self
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

pub fn view() -> Element<'static, Msg> {
    container(text("Queue").size(16))
        .padding(8)
        .width(Fill)
        .height(Fill)
        .into()
}
