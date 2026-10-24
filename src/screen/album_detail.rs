// TODO(ui_01 phase 3): implement `Screen` for `State` (init/update/view),
// following the `artists.rs` idiom, and hold it in `MainScreen`'s `SubScreen`
// enum. Placeholder screen until then.

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
    container(text("Album Detail").size(16))
        .padding(8)
        .width(Fill)
        .height(Fill)
        .into()
}
