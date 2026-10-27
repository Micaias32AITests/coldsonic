//! Set-library shell: list libraries, add one, open the set.
//!
//! A thin coordinator over two child screens — [`list_libraries`] (home)
//! and [`add_offline`] — plus the `Opening` in-flight state. The shell
//! loads `libraries.toml` once, routes events between the children, runs
//! `Manager::open` on "Open", and emits `Arc<Manager>` upward, after which
//! the coordinator swaps to the main screen. (Auto-open on boot is future
//! work.) Facet 3 adds a subsonic login mode next to `AddOffline` without
//! touching this shell.

use crate::config;
use crate::library::{LibraryConfig, Manager};
use crate::prelude::*;

pub mod add_offline;
pub mod list_libraries;

#[derive(Debug, Clone)]
pub struct State {
    mode: Mode,
}

#[derive(Debug, Clone)]
enum Mode {
    /// Home: the library list.
    ListLibraries(list_libraries::State),
    AddOffline(add_offline::State),
    /// In-flight state while `Manager::open` runs after "Open"; carries the
    /// list so a failure can restore it beside the error.
    Opening(Vec<LibraryConfig>),
}

#[derive(Debug, Clone)]
pub enum Msg {
    ListLibraries(list_libraries::Msg),
    AddOffline(add_offline::Msg),
    /// Internal: the list asked for the form.
    ShowAddForm(Vec<LibraryConfig>),
    /// Internal: the form exited (saved or backed out) — back to the list.
    ShowList(Vec<LibraryConfig>),
    /// Internal: the list asked to open the app.
    StartOpening(Vec<LibraryConfig>),
    OpenFinished(Result<Arc<Manager>, String>),
}

fn open_task(libraries: Vec<LibraryConfig>) -> Action<Msg, Arc<Manager>> {
    Action::task(Task::perform(
        async move {
            Manager::open(&libraries)
                .await
                .map(Arc::new)
                .map_err(|err| err.to_string())
        },
        Msg::OpenFinished,
    ))
}

impl State {
    pub fn init() -> (Self, Task<Msg>) {
        let (libraries, error) = match config::load_libraries() {
            Ok(libraries) => (libraries, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let list = list_libraries::State::init(libraries, error);
        (
            Self {
                mode: Mode::ListLibraries(list),
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, msg: Msg) -> Action<Msg, Arc<Manager>> {
        match msg {
            Msg::ListLibraries(inner) => {
                let Mode::ListLibraries(screen) = &mut self.mode else {
                    return Action::none();
                };
                Action::task(screen.update(inner).handle(Msg::ListLibraries, |event| {
                    Task::done(match event {
                        list_libraries::Event::OpenApp(libraries) => Msg::StartOpening(libraries),
                        list_libraries::Event::AddRequested(libraries) => {
                            Msg::ShowAddForm(libraries)
                        }
                    })
                }))
            }
            Msg::AddOffline(inner) => {
                let Mode::AddOffline(screen) = &mut self.mode else {
                    return Action::none();
                };
                Action::task(screen.update(inner).handle(Msg::AddOffline, |event| {
                    Task::done(match event {
                        add_offline::Event::Back(libraries)
                        | add_offline::Event::Saved(libraries) => Msg::ShowList(libraries),
                    })
                }))
            }
            Msg::ShowAddForm(libraries) => {
                let form = add_offline::State::init(libraries);
                self.mode = Mode::AddOffline(form);
                Action::none()
            }
            Msg::ShowList(libraries) => {
                let list = list_libraries::State::init(libraries, None);
                self.mode = Mode::ListLibraries(list);
                Action::none()
            }
            Msg::StartOpening(libraries) => {
                if !libraries.iter().any(LibraryConfig::enabled) {
                    return Action::none();
                }
                let task = open_task(libraries.clone());
                self.mode = Mode::Opening(libraries);
                task
            }
            Msg::OpenFinished(result) => {
                let libraries = match &self.mode {
                    Mode::Opening(libraries) => libraries.clone(),
                    _ => Vec::new(),
                };
                match result {
                    Ok(manager) => Action::emit(manager),
                    Err(err) => {
                        warn!(error = %err, "library open failed");
                        let list = list_libraries::State::init(libraries, Some(err));
                        self.mode = Mode::ListLibraries(list);
                        Action::none()
                    }
                }
            }
        }
    }

    pub fn view(&self) -> iced::Element<'_, Msg> {
        match &self.mode {
            Mode::ListLibraries(screen) => screen.view().map(Msg::ListLibraries),
            Mode::AddOffline(screen) => screen.view().map(Msg::AddOffline),
            Mode::Opening(_) => container(
                widget::column![text("Opening…")]
                    .spacing(12)
                    .align_x(Center),
            )
            .center(Fill)
            .into(),
        }
    }
}
