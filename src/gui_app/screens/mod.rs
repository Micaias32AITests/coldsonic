use gpui_kit::{AnyElement, Hsla, ObjectFit, Pixels, SharedString, div, img, prelude::*};

use crate::{
    gui_app::{
        RT,
        cover::{Cover, CoverCache, SMALL_COVER_PX, decode},
    },
    library::{CoverId, ManagerHandle},
};

pub mod album_detail;
pub mod albums;
pub mod artist_detail;
pub mod artists;
pub mod tracks;

#[derive(Default)]
pub enum LoadState<T> {
    #[default]
    Loading,
    Loaded(T),
    Failed(SharedString),
}

impl<T> LoadState<T> {
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(t) => Some(t),
            _ => None,
        }
    }

    /// text to show if the there is no state loaded yet, or if an error occurred. if `None`, that
    /// means it loaded.
    pub fn state_message(&self) -> Option<SharedString> {
        match self {
            Self::Loading => Some("Loading...".into()),
            Self::Failed(s) => Some(format!("Error: {s}").into()),
            Self::Loaded(_) => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }
}

impl<T, E: std::fmt::Display> From<Result<T, E>> for LoadState<T> {
    fn from(value: Result<T, E>) -> Self {
        match value {
            Ok(t) => Self::Loaded(t),
            Err(e) => Self::Failed(e.to_string().into()),
        }
    }
}

impl<T> LoadState<Vec<T>> {
    pub fn items(&self) -> &[T] {
        match self {
            Self::Loaded(v) => v.as_slice(),
            _ => &[],
        }
    }
}

// insane function signature btw
pub fn spawn_load_on_library<T, V>(
    cx: &mut Context<T>,
    fetch: impl Future<Output = Result<V, crate::library::LibraryError>> + Send + 'static,
    apply: impl FnOnce(&mut T, LoadState<V>) + 'static,
) where
    T: 'static,
    V: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let rt = cx.global::<RT>().get();

    rt.spawn(async move {
        let _ = tx.send(fetch.await);
    });

    cx.spawn(async move |this, cx| {
        let Ok(res) = rx.await else {
            return;
        };

        this.update(cx, |this, cx| {
            apply(this, LoadState::from(res));
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn cover_cell(cover: Option<&Cover>, size: Pixels, placeholder_color: Hsla) -> AnyElement {
    let fallback = move || {
        div()
            .size(size)
            .rounded_md()
            .bg(placeholder_color)
            .into_any_element()
    };

    match cover {
        Some(Cover::Loaded(src)) => img(src.clone())
            .size(size)
            .rounded_md()
            .object_fit(ObjectFit::Cover)
            .with_fallback(fallback)
            .into_any_element(),
        _ => fallback(),
    }
}

pub fn placeholder(message: impl Into<SharedString>) -> AnyElement {
    div()
        .size_full()
        .content_center()
        .child(message.into())
        .into_any_element()
}

pub trait HasCovers {
    fn covers(&mut self) -> &mut CoverCache;
}

pub fn ensure_cover<T: HasCovers + 'static>(
    covers: &mut CoverCache,
    manager: ManagerHandle,
    cx: &mut Context<T>,
    id: &CoverId,
) {
    if !covers.start_fetch(id) {
        return;
    }
    let fetch_id = id.clone();

    let (tx, rx) = tokio::sync::oneshot::channel();
    let rt = cx.global::<RT>().get();

    rt.spawn(async move {
        let _ = tx.send(manager.cover(&fetch_id, SMALL_COVER_PX).await);
    });

    let cover_id = id.clone();
    cx.spawn(async move |this, cx| {
        let Ok(result) = rx.await else { return };

        let cover = match result {
            Ok(bytes) => decode(&bytes),
            Err(err) => {
                tracing::debug!(%err, "cover fetch failed");
                Cover::Failed
            }
        };

        this.update(cx, |this, cx| {
            if this.covers().end_fetch(cover_id, cover) {
                cx.notify()
            }
        })
        .ok();
    })
    .detach();
}
