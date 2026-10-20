use iced::{Element, Subscription, Task, Theme, application};

use coldsonic::screen;
use coldsonic::screen::library_setup;
use coldsonic::{Screen, library::Manager};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    application(State::init, State::update, State::view)
        .subscription(State::subscription)
        .theme(Theme::CatppuccinMocha)
        .run()?;
    Ok(())
}

#[derive(Debug)]
pub enum State {
    Setup(library_setup::State),
    // boxed cuz clippy is a bitch (a good one)
    Main(Box<screen::MainScreen>),
}

#[derive(Debug, Clone)]
pub enum Msg {
    ScreenSetup(library_setup::Msg),
    // clipppy being a bitch again
    ScreenMain(Box<screen::Msg>),
    LibraryReady(Arc<Manager>),
    FontLoaded(Result<(), iced::font::Error>),
}

impl State {
    pub fn init() -> (Self, Task<Msg>) {
        let (screen, action) = library_setup::State::init(());
        let setup_task = action.handle(Msg::ScreenSetup, |manager| {
            Task::done(Msg::LibraryReady(manager))
        });
        let task = Task::batch(vec![
            setup_task,
            iced::font::load(iced_fonts::LUCIDE_FONT_BYTES).map(Msg::FontLoaded),
        ]);
        (Self::Setup(screen), task)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        match self {
            Self::Setup(screen) => screen.subscription().map(Msg::ScreenSetup),
            Self::Main(screen) => screen
                .subscription()
                .map(|msg| Msg::ScreenMain(Box::new(msg))),
        }
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::FontLoaded(_) => Task::none(),
            Msg::ScreenSetup(inner) => {
                let screen = match self {
                    Self::Setup(screen) => screen,
                    Self::Main(_) => return Task::none(),
                };
                screen.update(inner).handle(Msg::ScreenSetup, |manager| {
                    Task::done(Msg::LibraryReady(manager))
                })
            }
            Msg::LibraryReady(manager) => {
                let (screen, action) = screen::MainScreen::init(manager);
                *self = Self::Main(Box::new(screen));
                action.handle(|msg| Msg::ScreenMain(Box::new(msg)), |_| Task::none())
            }
            Msg::ScreenMain(inner) => {
                let screen = match self {
                    Self::Main(screen) => screen,
                    Self::Setup(_) => return Task::none(),
                };
                screen
                    .update(*inner)
                    .handle(|msg| Msg::ScreenMain(Box::new(msg)), |_| Task::none())
            }
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        match self {
            Self::Setup(screen) => screen.view().map(Msg::ScreenSetup),
            Self::Main(screen) => screen.view().map(|msg| Msg::ScreenMain(Box::new(msg))),
        }
    }
}
