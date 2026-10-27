// TODO(ui_01 phase 4): implement inherent `init`/`update`/`view` for `State`,
// reusing the now-playing bar components. Held by the right panel.
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
    container(text("Now Playing").size(16))
        .padding(8)
        .width(Fill)
        .height(Fill)
        .into()
}
