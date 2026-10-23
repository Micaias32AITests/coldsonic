//! A stack of screens that only builds, lays out and draws the top entry.
//!
//! The widget owns the [`Element`] for the top entry, built once per frame by
//! the caller's view function. Every other entry keeps its widget [`Tree`]
//! (scroll offsets, ids, ...) stashed in the stack's own state, so popping back
//! to a buried screen restores its widget state without ever rebuilding it
//! while it is not visible.
//!
//! Entries are identified by a caller-provided unique id, not by their value:
//! re-pushing the same route creates a fresh entry (and a fresh tree), while a
//! surviving entry keeps its state across frames.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::widget::{Operation, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Size, mouse};

/// One item in a [`Stack`]: a stable per-push `id` plus the caller's data.
#[derive(Debug)]
pub struct StackEntry<T> {
    pub id: u64,
    pub value: T,
}

impl<T> StackEntry<T> {
    pub fn new(id: u64, value: T) -> Self {
        Self { id, value }
    }
}

#[derive(Debug)]
struct StackState {
    /// Id of the entry currently in `tree.children[0]`.
    top_id: Option<u64>,
    /// Retained trees for every entry below the top, oldest first.
    buried: Vec<(u64, Tree)>,
}

pub struct Stack<'a, T, Message, Theme, Renderer> {
    entries: &'a [StackEntry<T>],
    /// Element for the top entry. Buried entries are never built.
    top: Option<Element<'a, Message, Theme, Renderer>>,
    width: Length,
    height: Length,
}

impl<'a, T, Message, Theme, Renderer> Stack<'a, T, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    pub fn new<F>(entries: &'a [StackEntry<T>], view: F) -> Self
    where
        F: FnOnce(&'a T) -> Element<'a, Message, Theme, Renderer>,
    {
        Self {
            entries,
            top: entries.last().map(|entry| view(&entry.value)),
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

impl<'a, T, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Stack<'a, T, Message, Theme, Renderer>
where
    T: 'a,
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<StackState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(StackState {
            top_id: None,
            buried: Vec::new(),
        })
    }

    fn children(&self) -> Vec<Tree> {
        self.top.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        let desired = self.entries.last().map(|entry| entry.id);
        let state = tree.state.downcast_mut::<StackState>();

        // Forget trees whose entry no longer exists.
        state
            .buried
            .retain(|(id, _)| self.entries.iter().any(|entry| entry.id == *id));

        let old_top = state.top_id;
        let Some(desired) = desired else {
            tree.children.clear();
            state.top_id = None;
            return;
        };

        if old_top == Some(desired) {
            if let (Some(top), Some(child)) = (&self.top, tree.children.first_mut()) {
                child.diff(top);
            } else if let Some(top) = &self.top {
                tree.children.push(Tree::new(top));
            }
            return;
        }

        // The top changed. Keep the previous top's tree if it is still in the
        // stack (a push); otherwise it was popped/reset and can be dropped.
        if let Some(old_top_id) = old_top
            && let Some(child) = tree.children.pop()
            && self.entries.iter().any(|entry| entry.id == old_top_id)
        {
            state.buried.retain(|(id, _)| *id != old_top_id);
            state.buried.push((old_top_id, child));
        }

        let Some(top) = &self.top else {
            tree.children.clear();
            state.top_id = Some(desired);
            return;
        };

        // Coming back to a buried entry (pop) reuses its tree; a brand new
        // entry starts with a fresh one.
        let mut child = match state.buried.iter().position(|(id, _)| *id == desired) {
            Some(index) => state.buried.remove(index).1,
            None => Tree::new(top),
        };
        child.diff(top);

        tree.children.clear();
        tree.children.push(child);
        state.top_id = Some(desired);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);

        let Some(top) = self.top.as_mut() else {
            return layout::Node::new(limits.resolve(self.width, self.height, Size::ZERO));
        };
        let Some(child_tree) = tree.children.first_mut() else {
            return layout::Node::new(limits.resolve(self.width, self.height, Size::ZERO));
        };

        let child = top.as_widget_mut().layout(child_tree, renderer, &limits);
        layout::Node::with_children(
            limits.resolve(self.width, self.height, child.size()),
            vec![child],
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let (Some(top), Some(child_tree), Some(child_layout)) = (
            self.top.as_mut(),
            tree.children.first_mut(),
            layout.children().next(),
        ) else {
            return;
        };

        top.as_widget_mut()
            .operate(child_tree, child_layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let (Some(top), Some(child_tree), Some(child_layout)) = (
            self.top.as_mut(),
            tree.children.first_mut(),
            layout.children().next(),
        ) else {
            return;
        };

        top.as_widget_mut().update(
            child_tree,
            event,
            child_layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let (Some(top), Some(child_tree), Some(child_layout)) = (
            self.top.as_ref(),
            tree.children.first(),
            layout.children().next(),
        ) else {
            return;
        };

        top.as_widget().draw(
            child_tree,
            renderer,
            theme,
            style,
            child_layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let (Some(top), Some(child_tree), Some(child_layout)) = (
            self.top.as_ref(),
            tree.children.first(),
            layout.children().next(),
        ) else {
            return mouse::Interaction::default();
        };

        top.as_widget()
            .mouse_interaction(child_tree, child_layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        let (Some(top), Some(child_tree), Some(child_layout)) = (
            self.top.as_mut(),
            tree.children.first_mut(),
            layout.children().next(),
        ) else {
            return None;
        };

        top.as_widget_mut()
            .overlay(child_tree, child_layout, renderer, viewport, translation)
    }
}

impl<'a, T, Message, Theme, Renderer> From<Stack<'a, T, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    T: 'a,
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(stack: Stack<'a, T, Message, Theme, Renderer>) -> Self {
        Element::new(stack)
    }
}

pub fn stack<'a, T, Message, Theme, Renderer>(
    entries: &'a [StackEntry<T>],
    view: impl FnOnce(&'a T) -> Element<'a, Message, Theme, Renderer>,
) -> Stack<'a, T, Message, Theme, Renderer>
where
    T: 'a,
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    Stack::new(entries, view)
}
