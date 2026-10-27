//! `AddOffline` — the offline-library registration form.
//!
//! Full state machine with live validation; the view is the spec'd layout.
//! Saving persists the appended list and hands it back to the shell (which
//! returns to `ListLibraries`); backing out returns the untouched list.

use crate::config;
use crate::library::{LibraryConfig, ScanMode, slugify};
use crate::prelude::*;

#[derive(Debug, Clone)]
pub struct State {
    /// Snapshot of the config list for uniqueness validation; handed back to
    /// the shell on exit either way.
    libraries: Vec<LibraryConfig>,
    name: String,
    id: String,
    auto_id: bool,
    root: String,
    scan: ScanMode,
    /// Live validation / save error, rendered by the view.
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Noop,
    NameEdit(String),
    IdEdit(String),
    AutoIdToggled(bool),
    RootEdit(String),
    SelectRoot,
    ScanToggled(ScanMode),
    AttemptSave,
    /// Leave the form without saving.
    Back,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Leave the form without saving; carries the untouched list.
    Back(Vec<LibraryConfig>),
    /// Persisted the appended list; carries it back for `ListLibraries`.
    Saved(Vec<LibraryConfig>),
}

impl State {
    /// Live validation: after any form edit, recompute the error from the
    /// current field values. An all-empty (pristine) form shows no error yet.
    fn refresh_error(&mut self) {
        let State {
            libraries,
            name,
            id,
            root,
            error,
            ..
        } = self;
        *error = if name.is_empty() && id.is_empty() && root.is_empty() {
            None
        } else {
            validate(name, id, root, libraries)
        };
    }

    fn can_save(&self) -> bool {
        self.error.is_none()
            && !self.name.is_empty()
            && !self.id.is_empty()
            && !self.root.is_empty()
    }
}

impl State {
    pub fn init(libraries: Vec<LibraryConfig>) -> Self {
        Self {
            libraries,
            name: String::new(),
            id: String::new(),
            auto_id: true,
            root: String::new(),
            scan: ScanMode::default(),
            error: None,
        }
    }

    pub fn update(&mut self, msg: Msg) -> Action<Msg, Event> {
        match msg {
            Msg::Noop => Action::none(),
            Msg::NameEdit(value) => {
                self.name = value;
                if self.auto_id {
                    self.id = slugify(&self.name);
                }
                self.refresh_error();
                Action::none()
            }
            Msg::IdEdit(value) => {
                // Manual entry wins over the auto slug.
                self.id = value;
                self.auto_id = false;
                self.refresh_error();
                Action::none()
            }
            Msg::AutoIdToggled(on) => {
                self.auto_id = on;
                if on {
                    self.id = slugify(&self.name);
                }
                self.refresh_error();
                Action::none()
            }
            Msg::RootEdit(value) => {
                self.root = value;
                self.refresh_error();
                Action::none()
            }
            Msg::SelectRoot => {
                info!("selecting root folder");
                Action::task(Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    |r| match r {
                        Some(path) => Msg::RootEdit(path.to_string_lossy().to_string()),
                        None => Msg::Noop,
                    },
                ))
            }
            Msg::ScanToggled(scan) => {
                self.scan = scan;
                self.refresh_error();
                Action::none()
            }
            Msg::AttemptSave => {
                if let Some(err) = validate(&self.name, &self.id, &self.root, &self.libraries) {
                    self.error = Some(err);
                    return Action::none();
                }
                let config = LibraryConfig::Offline {
                    name: self.name.clone(),
                    id: self.id.clone(),
                    root: self.root.clone().into(),
                    scan: self.scan,
                    enabled: true,
                    never_split: Vec::new(),
                };
                let added_id = config.identifier().to_string();
                let mut libraries = self.libraries.clone();
                libraries.push(config);
                if let Err(err) = config::save_libraries(&libraries) {
                    warn!(error = %err, "failed to save libraries.toml");
                    self.error = Some(err.to_string());
                    return Action::none();
                }
                info!(id = %added_id, "library added");
                Action::emit(Event::Saved(libraries))
            }
            Msg::Back => Action::emit(Event::Back(self.libraries.clone())),
        }
    }

    pub fn view(&self) -> iced::Element<'_, Msg> {
        let mut list = widget::column![
            text("Add Local Library").size(20),
            text_input("Name", &self.name).on_input(Msg::NameEdit),
            widget::row![
                text_input("Identifier", &self.id)
                    .on_input_maybe((!self.auto_id).then_some(Msg::IdEdit)),
                checkbox(self.auto_id)
                    .on_toggle(Msg::AutoIdToggled)
                    .label("Auto generate identifier")
            ]
            .align_y(Center)
            .spacing(5.0),
            widget::row![
                text_input("Library Root", &self.root),
                button("Select Library root").on_press(Msg::SelectRoot),
            ]
            .align_y(Center)
            .spacing(5.0),
            text("Scan mode"),
            pick_list(
                [ScanMode::Tags, ScanMode::Folders],
                Some(self.scan),
                Msg::ScanToggled
            ),
        ]
        .align_x(Center)
        .width(500.0)
        .spacing(8);
        if let Some(error) = &self.error {
            list = list.push(text(error).color([0.9, 0.3, 0.3]));
        }
        list = list.push(
            button("Add Offline Library")
                .on_press_maybe(self.can_save().then_some(Msg::AttemptSave)),
        );
        list = list.push(button("Back").style(button::secondary).on_press(Msg::Back));
        container(list).center(Fill).into()
    }
}

fn validate(name: &str, id: &str, root: &str, existing: &[LibraryConfig]) -> Option<String> {
    if name.trim().is_empty() {
        return Some("the name must not be empty".into());
    }
    if root.trim().is_empty() {
        return Some("the root path must not be empty".into());
    }
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Some(format!("identifier '{id}' must only contain [a-z0-9-]"));
    }
    if existing.iter().any(|lib| lib.identifier() == id) {
        return Some(format!("identifier '{id}' is already in use"));
    }
    None
}
