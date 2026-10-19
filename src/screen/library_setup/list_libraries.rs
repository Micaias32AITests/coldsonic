//! `ListLibraries` — the library list; home of the set-library shell.
//!
//! Shows every configured entry with enable/disable + delete, an "Add local
//! library" jump to the form, and an "Open" button that hands the whole list
//! upward (the shell runs `Manager::open` on it). Persists every mutation of
//! `libraries.toml` itself.

use crate::config;
use crate::library::LibraryConfig;
use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct State {
    libraries: Vec<LibraryConfig>,
    /// `libraries.toml` write failure, rendered under the list.
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// "Open" — open every enabled library.
    OpenApp,
    /// "Add local library" — ask the shell for the AddOffline form.
    AddRequested,
    /// Flip an entry's `enabled` flag and persist.
    ToggleEnabled(usize),
    /// Remove an entry and persist.
    DeleteRequested(usize),
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Open every enabled library; carries the list for `Manager::open`.
    OpenApp(Vec<LibraryConfig>),
    /// Switch to the AddOffline form; carries the current list for
    /// uniqueness validation.
    AddRequested(Vec<LibraryConfig>),
}

impl State {
    /// At least one entry is enabled — the "Open" button's gate.
    fn any_enabled(&self) -> bool {
        self.libraries.iter().any(LibraryConfig::enabled)
    }

    /// Write `libraries.toml` from the in-memory list; on failure surface the
    /// error in the view (the caller decides whether to roll its own change
    /// back).
    fn persist(&mut self) -> bool {
        match config::save_libraries(&self.libraries) {
            Ok(()) => true,
            Err(err) => {
                warn!(error = %err, "failed to save libraries.toml");
                self.error = Some(err.to_string());
                false
            }
        }
    }
}

impl Screen for State {
    type Msg = Msg;

    type Emit = Event;

    /// The loaded config list plus any load error to display.
    type InitData = (Vec<LibraryConfig>, Option<String>);

    fn init((libraries, error): Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        (Self { libraries, error }, Action::none())
    }

    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit> {
        match msg {
            Msg::OpenApp => {
                if !self.any_enabled() {
                    return Action::none();
                }
                Action::emit(Event::OpenApp(self.libraries.clone()))
            }
            Msg::AddRequested => Action::emit(Event::AddRequested(self.libraries.clone())),
            Msg::ToggleEnabled(index) => {
                let Some(config) = self.libraries.get_mut(index) else {
                    return Action::none();
                };
                let LibraryConfig::Offline { enabled, .. } = config;
                *enabled = !*enabled;
                let id = config.identifier();
                let enabled = config.enabled();
                info!(%id, enabled, "library enable flag toggled");
                self.persist();
                Action::none()
            }
            Msg::DeleteRequested(index) => {
                let Some(removed) = self.libraries.get(index).cloned() else {
                    return Action::none();
                };
                self.libraries.remove(index);
                info!(id = %removed.identifier(), "library deleted");
                if !self.persist() {
                    // Keep memory in sync with the file we failed to rewrite.
                    self.libraries.insert(index, removed);
                    return Action::none();
                }
                Action::none()
            }
        }
    }

    fn view(&self) -> iced::Element<'_, Self::Msg> {
        let mut list = widget::column![text("Libraries").size(20)]
            .spacing(8)
            .width(400);
        for (index, library) in self.libraries.iter().enumerate() {
            let name = if library.enabled() {
                library.name().to_string()
            } else {
                format!("{} (disabled)", library.name())
            };
            let toggle_label = if library.enabled() {
                "Disable"
            } else {
                "Enable"
            };
            list = list.push(
                widget::row![
                    text(name).width(Fill),
                    button(toggle_label).on_press(Msg::ToggleEnabled(index)),
                    button("Delete")
                        .style(button::danger)
                        .on_press(Msg::DeleteRequested(index)),
                ]
                .spacing(8),
            );
        }
        list = list.push(button("Add local library").on_press(Msg::AddRequested));
        list = list.push(button("Open").on_press_maybe(self.any_enabled().then_some(Msg::OpenApp)));
        if let Some(error) = &self.error {
            list = list.push(text(error).color([0.9, 0.3, 0.3]));
        }
        container(list.align_x(Center)).center(Fill).into()
    }
}
