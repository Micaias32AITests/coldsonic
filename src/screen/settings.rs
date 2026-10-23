// TODO(ui_01 phase 5): implement `Screen` for `State` (init/update/view) and
// push it onto the navigation stack on top of the current route. Placeholder
// screen until then.

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
    container(text("Settings").size(16))
        .padding(8)
        .width(Fill)
        .height(Fill)
        .into()
}
