use async_compat::Compat;
use iced::{
    Alignment::Center,
    Length::Fill,
    Task,
    widget::{button, column, container, text_input},
};
use opensubsonic::{Auth, Client};

use crate::{Action, Screen, config::Credentials};

#[derive(Debug, Clone)]
pub struct State {
    url_field: String,
    user_field: String,
    password_field: String,
    logging_in: bool,
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

fn login_task(url: String, username: String, password: String) -> Action<Msg, Client> {
    Action::task(Task::perform(
        async move {
            let client = match Client::new(
                &url,
                Auth::Token {
                    username,
                    password,
                },
            ) {
                Ok(client) => client,
                Err(_) => return Err(()),
            };
            match Compat::new(client.ping()).await {
                Ok(()) => Ok(client),
                Err(_e) => Err(()),
            }
        },
        |res| match res {
            Ok(client) => Msg::LoginSucessful(client),
            Err(_e) => Msg::LoginFailed,
        },
    ))
}

impl Screen for State {
    type Msg = Msg;

    type Emit = Client;

    type InitData = Option<Credentials>;

    fn init(data: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        match data {
            Some(Credentials {
                url,
                username,
                password,
            }) => (
                Self {
                    url_field: url.clone(),
                    user_field: username.clone(),
                    password_field: password.clone(),
                    logging_in: true,
                },
                login_task(url, username, password),
            ),
            None => (
                Self {
                    url_field: String::new(),
                    user_field: String::new(),
                    password_field: String::new(),
                    logging_in: false,
                },
                Action::none(),
            ),
        }
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
                    ..
                } = self.clone();
                self.logging_in = true;
                return login_task(url_field, user_field, password_field);
            }
            Msg::LoginSucessful(client) => return Action::Emit(client),
            Msg::LoginFailed => {
                self.logging_in = false;
                eprintln!("[login] failed to log in");
            }
        }
        Action::none()
    }

    fn view(&self) -> iced::Element<'_, Self::Msg> {
        let Self {
            url_field,
            user_field,
            password_field,
            logging_in,
        } = self;
        container(
            column![
                text_input("URL", url_field).on_input(Msg::UrlEdit),
                text_input("Username", user_field).on_input(Msg::UserEdit),
                text_input("Password", password_field)
                    .secure(true)
                    .on_input(Msg::PasswordEdit),
                if *logging_in {
                    button("Logging in…").on_press_maybe(None)
                } else {
                    button("Log In").on_press(Msg::AttemptLogin)
                }
            ]
            .width(500.0)
            .align_x(Center),
        )
        .center(Fill)
        .into()
    }
}
