//! Library file watcher and manager-event subscription.

use std::any::TypeId;
use std::hash::Hash;
use std::path::PathBuf;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, new_debouncer};

use crate::library::{ManagerEvent, ManagerHandle};
use crate::prelude::*;
use futures::SinkExt;
use iced::Subscription;

/// Quiet period after the last filesystem event before a scan is requested.
const DEBOUNCE: Duration = Duration::from_secs(1);

/// Watch one library root recursively and emit its identifier after a quiet
/// period. DB/cache directories live outside the root, so the app never reacts
/// to its own writes.
pub fn watcher_stream(
    (identifier, root): &(String, PathBuf),
) -> iced::futures::stream::BoxStream<'static, String> {
    let identifier = identifier.clone();
    let root = root.clone();
    Box::pin(iced::stream::channel(
        16,
        move |mut output: iced::futures::channel::mpsc::Sender<String>| async move {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
            let debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
                if let Ok(events) = result
                    && !events.is_empty()
                {
                    let _ = tx.send(());
                }
            });

            let mut debouncer = match debouncer {
                Ok(debouncer) => debouncer,
                Err(err) => {
                    warn!(%identifier, %err, "file watcher unavailable; using startup checks only");
                    return;
                }
            };
            if let Err(err) = debouncer.watch(&root, RecursiveMode::Recursive) {
                warn!(%identifier, root = %root.display(), %err, "cannot watch library root");
                return;
            }

            while rx.recv().await.is_some() {
                if output.send(identifier.clone()).await.is_err() {
                    break;
                }
            }
        },
    ))
}

/// Build a subscription from the manager's broadcast receiver.
pub fn manager_events(
    receiver: tokio::sync::broadcast::Receiver<ManagerEvent>,
) -> Subscription<ManagerEvent> {
    iced_futures::subscription::from_recipe(ManagerEvents(receiver))
}

struct ManagerEvents(tokio::sync::broadcast::Receiver<ManagerEvent>);

impl iced_futures::subscription::Recipe for ManagerEvents {
    type Output = ManagerEvent;

    fn hash(&self, state: &mut iced_futures::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(
        self: Box<Self>,
        _input: iced_futures::subscription::EventStream,
    ) -> futures::stream::BoxStream<'static, Self::Output> {
        Box::pin(futures::stream::unfold(self.0, |mut receiver| async move {
            match receiver.recv().await {
                Ok(event) => Some((event, receiver)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    Some((ManagerEvent::Lagged, receiver))
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
            }
        }))
    }
}

/// Plans used by the watcher subscription. Kept in one place so the screen
/// does not need to know how a plan is built.
pub fn watchers(handle: &ManagerHandle) -> Vec<Subscription<String>> {
    handle
        .watchers()
        .into_iter()
        .map(|(identifier, root)| Subscription::run_with((identifier, root), watcher_stream))
        .collect()
}
