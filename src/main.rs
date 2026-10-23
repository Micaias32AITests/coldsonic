#[cfg(feature = "dev")]
use iced::Alignment::Center;
use iced::Length::Fill;
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
    /// Debug playground — the first screen when the `dev` feature is on.
    #[cfg(feature = "dev")]
    Debug(screen::debug::State),
    Setup(library_setup::State),
    // boxed cuz clippy is a bitch (a good one)
    Main(Box<screen::MainScreen>),
}

#[derive(Debug, Clone)]
pub enum Msg {
    #[cfg(feature = "dev")]
    ScreenDebug(screen::debug::Msg),
    #[cfg(feature = "dev")]
    GoToDebug,
    GoToSetup,
    ScreenSetup(library_setup::Msg),
    // clipppy being a bitch again
    ScreenMain(Box<screen::Msg>),
    LibraryReady(Arc<Manager>),
    FontLoaded(Result<(), iced::font::Error>),
}

impl State {
    pub fn init() -> (Self, Task<Msg>) {
        // The `dev` build boots into the playground: a widget can be tested
        // without a configured library.
        #[cfg(feature = "dev")]
        {}

        {
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
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        match self {
            #[cfg(feature = "dev")]
            Self::Debug(screen) => screen.subscription().map(Msg::ScreenDebug),
            Self::Setup(screen) => screen.subscription().map(Msg::ScreenSetup),
            Self::Main(screen) => screen
                .subscription()
                .map(|msg| Msg::ScreenMain(Box::new(msg))),
        }
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::FontLoaded(_) => Task::none(),
            #[cfg(feature = "dev")]
            Msg::ScreenDebug(inner) => {
                let action = match self {
                    Self::Debug(screen) => screen.update(inner),
                    _ => return Task::none(),
                };

                action.handle(Msg::ScreenDebug, handle_debug_emit)
            }
            #[cfg(feature = "dev")]
            Msg::GoToDebug => {
                let (screen, action) = screen::debug::State::init(());
                let task = action.handle(Msg::ScreenDebug, |_event| Task::none());

                *self = Self::Debug(screen);
                Task::batch(vec![
                    task,
                    iced::font::load(iced_fonts::LUCIDE_FONT_BYTES).map(Msg::FontLoaded),
                ])
            }
            Msg::GoToSetup => {
                // Back to the normal flow.
                let (screen, setup_action) = library_setup::State::init(());
                *self = Self::Setup(screen);
                setup_action.handle(Msg::ScreenSetup, |manager| {
                    Task::done(Msg::LibraryReady(manager))
                })
            }
            Msg::ScreenSetup(inner) => {
                let screen = match self {
                    Self::Setup(screen) => screen,
                    Self::Main(_) => return Task::none(),
                    #[cfg(feature = "dev")]
                    Self::Debug(_) => return Task::none(),
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
                    #[cfg(feature = "dev")]
                    Self::Debug(_) => return Task::none(),
                };
                screen
                    .update(*inner)
                    .to_task()
                    .map(|msg| Msg::ScreenMain(Box::new(msg)))
            }
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        iced::widget::column([
            match self {
                Self::Setup(screen) => screen.view().map(Msg::ScreenSetup),
                Self::Main(screen) => screen.view().map(|msg| Msg::ScreenMain(Box::new(msg))),
                #[cfg(feature = "dev")]
                Self::Debug(screen) => screen.view().map(Msg::ScreenDebug),
            },
            #[cfg(feature = "dev")]
            iced::widget::row![iced::widget::button("Debug Screen").on_press(Msg::GoToDebug)]
                .align_y(Center)
                .into(),
        ])
        .height(Fill)
        .into()
    }
}

#[cfg(feature = "dev")]
fn handle_debug_emit(event: screen::debug::Event) -> Task<Msg> {
    match event {
        screen::debug::Event::Back => Task::done(Msg::GoToSetup),
    }
}
