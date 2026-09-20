use iced::{Element, Subscription, Task, application};
use opensubsonic::Client;

use coldsonic::screen;
use coldsonic::{Screen, config};

fn main() -> anyhow::Result<()> {
    application(State::init, State::update, State::view)
        .subscription(State::subscription)
        .run()?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum State {
    Login(screen::login::State),
    // boxed cuz clippy is a bitch (a good one)
    Main(Box<screen::MainScreen>),
}

#[derive(Debug, Clone)]
pub enum Msg {
    ScreenLogin(screen::login::Msg),
    // clipppy being a bitch again
    ScreenMain(Box<screen::Msg>),
    LoggedIn(Client),
    FontLoaded(Result<(), iced::font::Error>),
}

impl State {
    pub fn init() -> (Self, Task<Msg>) {
        let (screen, action) = screen::login::State::init(config::Credentials::load());
        let login_task = action.handle(Msg::ScreenLogin, |_| Task::none());
        let task = Task::batch(vec![
            login_task,
            iced::font::load(iced_fonts::LUCIDE_FONT_BYTES).map(Msg::FontLoaded),
        ]);
        (Self::Login(screen), task)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        match self {
            Self::Login(screen) => screen.subscription().map(Msg::ScreenLogin),
            Self::Main(screen) => screen
                .subscription()
                .map(|msg| Msg::ScreenMain(Box::new(msg))),
        }
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::FontLoaded(_) => Task::none(),
            Msg::ScreenLogin(inner) => {
                let screen = match self {
                    Self::Login(screen) => screen,
                    Self::Main(_) => return Task::none(),
                };
                screen
                    .update(inner)
                    .handle(Msg::ScreenLogin, |client| Task::done(Msg::LoggedIn(client)))
            }
            Msg::LoggedIn(client) => {
                let (screen, action) = screen::MainScreen::init(client);
                *self = Self::Main(Box::new(screen));
                action.handle(|msg| Msg::ScreenMain(Box::new(msg)), |_| Task::none())
            }
            Msg::ScreenMain(inner) => {
                let screen = match self {
                    Self::Main(screen) => screen,
                    Self::Login(_) => return Task::none(),
                };
                screen
                    .update(*inner)
                    .handle(|msg| Msg::ScreenMain(Box::new(msg)), |_| Task::none())
            }
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        match self {
            Self::Login(screen) => screen.view().map(Msg::ScreenLogin),
            Self::Main(screen) => screen.view().map(|msg| Msg::ScreenMain(Box::new(msg))),
        }
    }
}
