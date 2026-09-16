use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task, application,
    widget::{button, column, container, text, text_input},
};
use opensubsonic::{Auth, Client};

fn main() -> anyhow::Result<()> {
    application(State::init, State::update, State::view).run()?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum State {
    NotLoggedIn {
        url_field: String,
        user_field: String,
        password_field: String,
    },
    LoggedIn {
        client: Client,
    },
}

impl State {
    pub fn init() -> (Self, Task<Msg>) {
        (
            Self::NotLoggedIn {
                url_field: String::new(),
                user_field: String::new(),
                password_field: String::new(),
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::UrlEdit(new) => {
                if let Self::NotLoggedIn { url_field, .. } = self {
                    *url_field = new
                }
            }
            Msg::UserEdit(new) => {
                if let Self::NotLoggedIn { user_field, .. } = self {
                    *user_field = new
                }
            }
            Msg::PasswordEdit(new) => {
                if let Self::NotLoggedIn { password_field, .. } = self {
                    *password_field = new
                }
            }
            Msg::AttemptLogin => {
                if let Self::NotLoggedIn {
                    url_field,
                    user_field,
                    password_field,
                } = self.clone()
                {
                    return Task::perform(
                        async move {
                            Client::new(
                                &url_field,
                                Auth::Token {
                                    username: user_field,
                                    password: password_field,
                                },
                            )
                        },
                        |res| match res {
                            Ok(client) => Msg::LoginSucessful(client),
                            Err(_e) => Msg::LoginFailed,
                        },
                    );
                }
            }
            Msg::LoginSucessful(client) => {
                *self = State::LoggedIn { client };
            }
            Msg::LoginFailed => {
                eprintln!("FUCK");
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        match self {
            Self::NotLoggedIn {
                url_field,
                user_field,
                password_field: password,
            } => container(
                column![
                    text_input("URL", url_field).on_input(Msg::UrlEdit),
                    text_input("Username", user_field).on_input(Msg::UserEdit),
                    text_input("Password", password)
                        .secure(true)
                        .on_input(Msg::PasswordEdit),
                    button("Log In").on_press(Msg::AttemptLogin)
                ]
                .width(500.0)
                .align_x(Center),
            )
            .center(Fill)
            .into(),
            Self::LoggedIn { client: _ } => text("logged in").into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    UrlEdit(String),
    UserEdit(String),
    PasswordEdit(String),
    AttemptLogin,
    LoginSucessful(Client),
    LoginFailed,
}
