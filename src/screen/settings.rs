//! Settings root: panel behavior and placeholders for future settings.
//!
//! The screen owns no parent state. `view` receives the current `UiSettings`
//! and live panel flag; changes are emitted to `MainScreen`.

use crate::config::{PanelScreenConfig, PanelStartup, UiSettings};
use crate::prelude::*;

#[derive(Debug)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Msg {
    PanelStartupSelected(PanelStartup),
    PanelScreenSelected(PanelScreenConfig),
    SetPanelOpen(bool),
    RescanLibraries,
}

#[derive(Debug, Clone)]
pub enum Event {
    SetPanelStartup(PanelStartup),
    SetPanelScreen(PanelScreenConfig),
    SetPanelOpen(bool),
    RescanLibraries,
}

pub fn init() -> State {
    State
}

pub fn update(_state: &mut State, msg: Msg) -> Action<Msg, Event> {
    let event = match msg {
        Msg::PanelStartupSelected(value) => Event::SetPanelStartup(value),
        Msg::PanelScreenSelected(value) => Event::SetPanelScreen(value),
        Msg::SetPanelOpen(open) => Event::SetPanelOpen(open),
        Msg::RescanLibraries => Event::RescanLibraries,
    };
    Action::emit(event)
}

pub fn view<'a>(_state: &'a State, settings: &'a UiSettings, panel_open: bool) -> Element<'a, Msg> {
    widget::column![
        text("Settings").size(24),
        section("Right panel"),
        setting_row(
            "Startup behavior",
            pick_list(
                [
                    PanelStartup::PersistState,
                    PanelStartup::StartClosed,
                    PanelStartup::StartOpened,
                ],
                Some(settings.panel_startup.clone()),
                Msg::PanelStartupSelected,
            )
            .into(),
        ),
        setting_row(
            "Default screen",
            pick_list(
                [PanelScreenConfig::Queue, PanelScreenConfig::NowPlaying],
                Some(settings.panel_screen.clone()),
                Msg::PanelScreenSelected,
            )
            .into(),
        ),
        setting_row(
            "Show right panel",
            checkbox(panel_open).on_toggle(Msg::SetPanelOpen).into(),
        ),
        section("Appearance"),
        setting_row(
            "Theme",
            text("System (coming soon)")
                .size(12)
                .color([0.55, 0.55, 0.55])
                .into(),
        ),
        section("Libraries"),
        setting_row(
            "Rescan libraries",
            button("Rescan now").on_press(Msg::RescanLibraries).into(),
        ),
        text("Settings are saved automatically.")
            .size(11)
            .color([0.55, 0.55, 0.55]),
    ]
    .spacing(10)
    .padding(16)
    .width(Fill)
    .height(Fill)
    .into()
}

fn section<'a>(title: &'a str) -> Element<'a, Msg> {
    container(text(title).size(14).color([0.65, 0.65, 0.65]))
        .padding(Padding {
            top: 10.0,
            bottom: 2.0,
            ..Padding::default()
        })
        .into()
}

fn setting_row<'a>(label: &'a str, control: Element<'a, Msg>) -> Element<'a, Msg> {
    widget::row![
        text(label).width(180),
        widget::container(control).width(Fill),
    ]
    .align_y(Center)
    .spacing(12)
    .into()
}
