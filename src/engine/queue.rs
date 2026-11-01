//! Shared playback queue.
//!
//! Both engines embed a [`Queue`]; it owns ordering, the current entry, and the
//! history. Engines keep audio, position, volume, and playback state.
//!
//! `Prev` is an *engine-level* command because choosing between restarting the
//! current track and stepping back into history needs the playback position,
//! which the queue does not know about. The queue therefore exposes the two
//! behaviours separately: [`Queue::step_back`] mutates the queue, while a
//! restart only reads [`Queue::current`] and is handled by the engine.

use std::collections::VecDeque;

use super::Track;

/// Stable identity of a queue row.
///
/// Monotonic per [`Queue`] and never reused. Duplicate tracks therefore get
/// distinct ids, and future remove/reorder operations cannot silently retarget
/// a row. At the 5k-track scale a linear scan by id is trivial, so no secondary
/// index is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueEntryId(u64);

impl QueueEntryId {
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A track plus its stable row id.
#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub id: QueueEntryId,
    pub track: Track,
}

/// Ordering, current entry, and history for one engine.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    upcoming: VecDeque<QueueEntry>,
    /// Played entries, newest last — feeds `Prev`.
    history: VecDeque<QueueEntry>,
    current: Option<QueueEntry>,
    next_id: u64,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the queue with `tracks`, clear history, and make the first
    /// track current. Returns the new current entry, if any.
    pub fn replace(&mut self, tracks: impl IntoIterator<Item = Track>) -> Option<&QueueEntry> {
        self.upcoming.clear();
        self.history.clear();
        self.current = None;
        for track in tracks {
            let entry = self.make_entry(track);
            self.upcoming.push_back(entry);
        }
        self.advance()
    }

    /// Make `track` current immediately. The previous current moves to
    /// history; the upcoming queue is untouched.
    pub fn play_now(&mut self, track: Track) -> &QueueEntry {
        if let Some(current) = self.current.take() {
            self.history.push_back(current);
        }
        let entry = self.make_entry(track);
        self.current = Some(entry);
        self.current.as_ref().expect("just set current")
    }

    /// Append to the end of the upcoming queue without changing playback.
    pub fn enqueue(&mut self, track: Track) -> QueueEntryId {
        let entry = self.make_entry(track);
        let id = entry.id;
        self.upcoming.push_back(entry);
        id
    }

    /// Advance to the next upcoming entry. The current entry moves to history.
    /// Returns the new current, or `None` when the queue is exhausted.
    pub fn advance(&mut self) -> Option<&QueueEntry> {
        if let Some(current) = self.current.take() {
            self.history.push_back(current);
        }
        self.current = self.upcoming.pop_front();
        self.current.as_ref()
    }

    /// Step back into history: the current entry returns to the front of the
    /// upcoming queue and the newest history entry becomes current.
    ///
    /// Returns `None` (and mutates nothing) when there is no history.
    pub fn step_back(&mut self) -> Option<&QueueEntry> {
        let previous = self.history.pop_back()?;
        if let Some(current) = self.current.take() {
            self.upcoming.push_front(current);
        }
        self.current = Some(previous);
        self.current.as_ref()
    }

    /// Clear the current entry and history, keeping the upcoming queue. This
    /// mirrors the engine's `Stop` command.
    pub fn stop(&mut self) {
        self.current = None;
        self.history.clear();
    }

    /// Drop the current entry without touching upcoming or history. Used when
    /// playback fails.
    pub fn clear_current(&mut self) {
        self.current = None;
    }

    pub fn current(&self) -> Option<&QueueEntry> {
        self.current.as_ref()
    }

    /// Borrowed iteration over the upcoming entries, in playback order.
    ///
    /// `Clone + ExactSizeIterator` lets [`crate::widget::lazy_list::lazy_list`]
    /// consume it directly.
    pub fn upcoming(&self) -> std::collections::vec_deque::Iter<'_, QueueEntry> {
        self.upcoming.iter()
    }

    pub fn len(&self) -> usize {
        self.upcoming.len()
    }

    pub fn is_empty(&self) -> bool {
        self.upcoming.is_empty()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Find an upcoming entry by its stable id.
    #[allow(dead_code)]
    pub fn find(&self, id: QueueEntryId) -> Option<&QueueEntry> {
        self.upcoming.iter().find(|entry| entry.id == id)
    }

    fn make_entry(&mut self, track: Track) -> QueueEntry {
        let entry = QueueEntry {
            id: QueueEntryId(self.next_id),
            track,
        };
        self.next_id += 1;
        entry
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::library::TrackId;

    use super::*;

    fn track(name: &str) -> Track {
        Track {
            id: TrackId::new(name),
            title: name.to_owned(),
            artist: None,
            album: None,
            track_no: None,
            duration: Duration::from_secs(60),
            cover: None,
        }
    }

    fn titles(queue: &Queue) -> Vec<&str> {
        queue
            .upcoming()
            .map(|entry| entry.track.title.as_str())
            .collect()
    }

    #[test]
    fn replace_clears_history_and_makes_first_current() {
        let mut queue = Queue::new();
        queue.play_now(track("old"));
        queue.enqueue(track("stale"));

        queue.replace([track("a"), track("b")]);

        assert_eq!(queue.current().map(|e| e.track.title.as_str()), Some("a"));
        assert_eq!(titles(&queue), ["b"]);
        assert_eq!(queue.history_len(), 0);
    }

    #[test]
    fn enqueue_appends_without_changing_current() {
        let mut queue = Queue::new();
        queue.replace([track("a")]);

        queue.enqueue(track("b"));

        assert_eq!(queue.current().map(|e| e.track.title.as_str()), Some("a"));
        assert_eq!(titles(&queue), ["b"]);
    }

    #[test]
    fn play_now_changes_current_and_history_but_not_upcoming() {
        let mut queue = Queue::new();
        queue.replace([track("a"), track("b")]);

        queue.play_now(track("c"));

        assert_eq!(queue.current().map(|e| e.track.title.as_str()), Some("c"));
        assert_eq!(titles(&queue), ["b"]);
        assert_eq!(queue.history_len(), 1);
    }

    #[test]
    fn advance_moves_current_to_history() {
        let mut queue = Queue::new();
        queue.replace([track("a"), track("b")]);

        queue.advance();

        assert_eq!(queue.current().map(|e| e.track.title.as_str()), Some("b"));
        assert!(queue.is_empty());
        assert_eq!(queue.history_len(), 1);
    }

    #[test]
    fn step_back_returns_current_to_upcoming_front() {
        let mut queue = Queue::new();
        queue.replace([track("a"), track("b"), track("c")]);
        queue.advance(); // a in history, b current

        queue.step_back();

        assert_eq!(queue.current().map(|e| e.track.title.as_str()), Some("a"));
        assert_eq!(titles(&queue), ["b", "c"]);
        assert_eq!(queue.history_len(), 0);
    }

    #[test]
    fn step_back_without_history_is_a_no_op() {
        let mut queue = Queue::new();
        queue.replace([track("a"), track("b")]);

        assert!(queue.step_back().is_none());

        assert_eq!(queue.current().map(|e| e.track.title.as_str()), Some("a"));
        assert_eq!(titles(&queue), ["b"]);
    }

    #[test]
    fn restart_path_does_not_mutate_the_queue() {
        let mut queue = Queue::new();
        queue.replace([track("a"), track("b"), track("c")]);
        queue.advance(); // b current

        let current = queue.current().cloned().expect("current");

        // The engine's restart-current branch only reads the current entry.
        assert_eq!(current.track.title, "b");
        assert_eq!(titles(&queue), ["c"]);
        assert_eq!(queue.history_len(), 1);
    }

    #[test]
    fn stop_clears_current_and_history_but_keeps_upcoming() {
        let mut queue = Queue::new();
        queue.replace([track("a"), track("b")]);
        queue.advance(); // b current, a in history
        queue.enqueue(track("c"));

        queue.stop();

        assert!(queue.current().is_none());
        assert_eq!(queue.history_len(), 0);
        assert_eq!(titles(&queue), ["c"]);
    }

    #[test]
    fn duplicate_tracks_get_distinct_stable_ids() {
        let mut queue = Queue::new();
        queue.replace([track("same"), track("same")]);
        let first = queue.current().expect("current").id;
        let second = queue.upcoming().next().expect("second").id;
        assert_ne!(first, second);
        assert_eq!(queue.find(second).map(|e| e.id), Some(second));

        queue.advance();
        assert_eq!(queue.current().map(|entry| entry.id), Some(second));
        assert!(queue.find(second).is_none());
        assert!(queue.find(first).is_none());
        assert!(queue.find(QueueEntryId(99)).is_none());
    }

    #[test]
    fn ids_are_not_reused_after_replacing_the_queue() {
        let mut queue = Queue::new();
        queue.replace([track("a")]);
        let first = queue.current().expect("current").id;

        queue.replace([track("b")]);
        let second = queue.current().expect("current").id;

        assert_ne!(first, second);
    }
}
