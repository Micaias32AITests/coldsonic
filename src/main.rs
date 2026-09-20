use iced::{Element, Task, application};
use opensubsonic::Client;

use coldsonic::{Screen, config};
use coldsonic::screen;

fn main() -> anyhow::Result<()> {
    application(State::init, State::update, State::view).run()?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum State {
    Login(screen::login::State),
    Main(screen::MainScreen),
}

#[derive(Debug, Clone)]
pub enum Msg {
    ScreenLogin(screen::login::Msg),
    ScreenMain(screen::Msg),
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
                *self = Self::Main(screen);
                action.handle(Msg::ScreenMain, |_| Task::none())
            }
            Msg::ScreenMain(inner) => {
                let screen = match self {
                    Self::Main(screen) => screen,
                    Self::Login(_) => return Task::none(),
                };
                screen
                    .update(inner)
                    .handle(Msg::ScreenMain, |_| Task::none())
            }
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        match self {
            Self::Login(screen) => screen.view().map(Msg::ScreenLogin),
            Self::Main(screen) => screen.view().map(Msg::ScreenMain),
        }
    }
}
