use async_compat::Compat;
use iced::{
    Alignment::Center,
    Element,
    Length::Fill,
    Task, application,
    widget::{Column, button, column, container, scrollable, text, text_input},
};
use opensubsonic::{Auth, Client, data::ArtistId3};

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
        artists: Vec<ArtistId3>,
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
                let client_for_load = client.clone();
                *self = State::LoggedIn {
                    client,
                    artists: Vec::new(),
                };
                return Task::perform(
                    Compat::new(async move { client_for_load.get_artists(None).await }),
                    |res| match res {
                        Ok(artists) => Msg::ArtistsLoaded(
                            artists
                                .index
                                .into_iter()
                                .flat_map(|index| index.artist)
                                .collect(),
                        ),
                        Err(_) => Msg::ArtistsLoadFailed,
                    },
                );
            }
            Msg::ArtistsLoaded(artists) => {
                if let Self::LoggedIn {
                    artists: current, ..
                } = self
                {
                    *current = artists;
                }
            }
            Msg::ArtistsLoadFailed => {
                eprintln!("FUCK");
            }
            Msg::ArtistClicked(_) => {}
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
            Self::LoggedIn { client: _, artists } => {
                if artists.is_empty() {
                    return text("loading...").into();
                }
                let list = Column::with_children(artists.iter().map(|artist| {
                    button(text(artist.name.clone()))
                        .width(Fill)
                        .on_press(Msg::ArtistClicked(artist.id.clone()))
                        .into()
                }))
                .spacing(4)
                .padding(8);
                scrollable(list).width(Fill).height(Fill).into()
            }
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
    ArtistsLoaded(Vec<ArtistId3>),
    ArtistsLoadFailed,
    ArtistClicked(String),
}
