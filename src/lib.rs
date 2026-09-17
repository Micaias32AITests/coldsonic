use iced::Subscription;

pub mod prelude;
pub mod screen;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Never {}

#[derive(Debug, Default)]
pub enum Action<T, U> {
    #[default]
    None,
    Task(iced::Task<T>),
    Emit(U),
}

impl<T, U> Action<T, U> {
    pub fn none() -> Self {
        Self::None
    }

    pub fn task(task: iced::Task<T>) -> Self {
        Self::Task(task)
    }

    pub fn emit(emit: U) -> Self {
        Self::Emit(emit)
    }

    pub fn handle<V>(
        self,
        map_msg: impl Fn(T) -> V + Send + 'static,
        on_emit: impl Fn(U) -> iced::Task<V> + Send + 'static,
    ) -> iced::Task<V>
    where
        T: Send + 'static,
        V: Send + 'static,
    {
        match self {
            Self::None => iced::Task::none(),
            Self::Task(task) => task.map(map_msg),
            Self::Emit(emit) => on_emit(emit),
        }
    }

    pub fn map_to_task<V>(self, mapper: impl Fn(Self) -> iced::Task<V>) -> iced::Task<V> {
        mapper(self)
    }

    pub fn map_task<V>(self, mapper: impl Fn(iced::Task<T>) -> iced::Task<V>) -> Action<V, U> {
        match self {
            Self::None => Action::None,
            Self::Task(t) => Action::Task(mapper(t)),
            Self::Emit(u) => Action::Emit(u),
        }
    }

    pub fn map<V, W>(self, mapper: impl Fn(Self) -> Action<V, W>) -> Action<V, W> {
        mapper(self)
    }
}

pub trait Screen {
    type Msg;

    /// this is the type that the upper recieves as a way to send messages to the parent screen
    type Emit;

    type InitData;

    #[must_use]
    fn init(data: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized;

    #[must_use]
    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit>;

    #[must_use]
    fn view(&self) -> iced::Element<'_, Self::Msg>;

    fn subscription(&self) -> Subscription<Self::Msg> {
        Subscription::none()
    }
}
