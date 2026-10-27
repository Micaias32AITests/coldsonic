use std::time::Duration;

use iced::stream;
use iced::{Element, Task};
use smol::stream::Stream;

use crate::Action;
use crate::prelude::*;
use crate::widget::lazy_list::lazy_list;

#[derive(Debug)]
pub struct State {
    list: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    AddNewValues(Vec<String>),
    GoBack,
}

#[derive(Debug, Clone)]
pub enum Event {
    Back,
}

impl State {
    pub fn init() -> (Self, Task<Msg>) {
        (State { list: Vec::new() }, Task::stream(list_grower()))
    }

    pub fn update(&mut self, msg: Msg) -> Action<Msg, Event> {
        match msg {
            Msg::AddNewValues(vals) => self.list.extend(vals),
            Msg::GoBack => return Action::emit(Event::Back),
        }

        Action::none()
    }

    pub fn view(&self) -> Element<'_, Msg> {
        widget::column![
            button("Go Back").on_press(Msg::GoBack),
            lazy_list(&self.list, 40.0, |_i, val| button(val.as_str()).into())
        ]
        .padding(25.0)
        .spacing(10.0)
        .into()
    }
}

fn list_grower() -> impl Stream<Item = Msg> {
    stream::channel(
        100,
        |mut out: iced::futures::channel::mpsc::Sender<Msg>| async move {
            for val in (1..).step_by(50) {
                smol::Timer::after(Duration::from_secs(1)).await;
                let vec = (val..val + 50).map(|v| format!("{v} item")).collect();
                let _ = out.try_send(Msg::AddNewValues(vec));
            }
        },
    )
}
