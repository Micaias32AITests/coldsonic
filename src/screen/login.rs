use iced::{
    Alignment::Center,
    Length::Fill,
    Task,
    widget::{button, column, container, text_input},
};
use opensubsonic::{Auth, Client};

use crate::{Action, Screen};

#[derive(Debug, Clone)]
pub struct State {
    url_field: String,
    user_field: String,
    password_field: String,
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

impl Screen for State {
    type Msg = Msg;

    type Emit = Client;

    type InitData = ();

    fn init((): Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        (
            Self {
                url_field: String::new(),
                user_field: String::new(),
                password_field: String::new(),
            },
            Action::none(),
        )
    }

    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit> {
        match msg {
            Msg::UrlEdit(new) => self.url_field = new,
            Msg::UserEdit(new) => self.user_field = new,
            Msg::PasswordEdit(new) => self.password_field = new,
            Msg::AttemptLogin => {
                let Self {
                    url_field,
                    user_field,
                    password_field,
                } = self.clone();
                return Action::task(Task::perform(
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
                ));
            }
            Msg::LoginSucessful(client) => return Action::Emit(client),
            Msg::LoginFailed => {
                eprintln!("FUCK");
            }
        }
        Action::none()
    }

    fn view(&self) -> iced::Element<'_, Self::Msg> {
        let Self {
            url_field,
            user_field,
            password_field,
        } = self;
        container(
            column![
                text_input("URL", url_field).on_input(Msg::UrlEdit),
                text_input("Username", user_field).on_input(Msg::UserEdit),
                text_input("Password", password_field)
                    .secure(true)
                    .on_input(Msg::PasswordEdit),
                button("Log In").on_press(Msg::AttemptLogin)
            ]
            .width(500.0)
            .align_x(Center),
        )
        .center(Fill)
        .into()
    }
}
