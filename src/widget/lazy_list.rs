//! A virtualized (lazy) list widget for iced 0.14.
//!
//! Only the rows intersecting the visible area (plus a little overscan) are
//! built, laid out, updated and drawn. Every row has the same fixed height, so
//! the visible range is simple arithmetic.
//!
//! Design: [`LazyList`] is a thin wrapper around iced's own `Scrollable`
//! (scrollbar, wheel, `on_scroll`, scroll operations all come for free). Its
//! content is a private widget, `Rows`, that is `len * item_height` tall but
//! only owns children for the visible window. `Rows` learns which window is
//! visible from the viewport that `Scrollable` passes to its `update`, and
//! asks for a relayout when the window changes.

use std::ops::Range;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::widget::Scrollable;
use iced::widget::scrollable::{self, Direction};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

pub use iced::widget::scrollable::Viewport;

/// Rows built before the first `update` tells us the real viewport size.
const INITIAL_ROWS: usize = 32;
/// Extra rows built above and below the visible area by default.
pub const DEFAULT_OVERSCAN: usize = 3;

/// Creates a [`LazyList`] with the default overscan.
///
/// * `source` must be cheap to clone and iterate repeatedly without being
///   consumed (e.g. `&Vec<T>`, `&[T]`), and its iterator must know its exact
///   length.
/// * `item_height` is the height in logical pixels of **every** row.
/// * `view_item` receives the absolute index and the item and builds its row.
///   Rows are laid out with a fixed size of `width × item_height`.
pub fn lazy_list<'a, I, Message, Theme, Renderer>(
    source: I,
    item_height: f32,
    view_item: impl Fn(usize, I::Item) -> Element<'a, Message, Theme, Renderer> + 'a,
) -> LazyList<'a, Message, Theme, Renderer>
where
    I: IntoIterator + Clone + 'a,
    I::IntoIter: ExactSizeIterator,
    Message: 'a,
    Theme: scrollable::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer + 'a,
{
    LazyList::new(source, item_height, view_item)
}

/// A scrollable list that only builds the rows that can be seen.
pub struct LazyList<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Theme: scrollable::Catalog,
    Renderer: iced::advanced::text::Renderer,
{
    scrollable: Scrollable<'a, Message, Theme, Renderer>,
}

impl<'a, Message, Theme, Renderer> LazyList<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: scrollable::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer + 'a,
{
    /// See [`lazy_list`].
    pub fn new<I>(
        source: I,
        item_height: f32,
        view_item: impl Fn(usize, I::Item) -> Element<'a, Message, Theme, Renderer> + 'a,
    ) -> Self
    where
        I: IntoIterator + Clone + 'a,
        I::IntoIter: ExactSizeIterator,
    {
        Self::with_overscan(source, item_height, DEFAULT_OVERSCAN, view_item)
    }

    /// Like [`LazyList::new`] but with a custom number of extra rows built
    /// above and below the visible area (reduces pop-in on fast scrolling).
    pub fn with_overscan<I>(
        source: I,
        item_height: f32,
        overscan: usize,
        view_item: impl Fn(usize, I::Item) -> Element<'a, Message, Theme, Renderer> + 'a,
    ) -> Self
    where
        I: IntoIterator + Clone + 'a,
        I::IntoIter: ExactSizeIterator,
    {
        assert!(
            item_height.is_finite() && item_height > 0.0,
            "LazyList item_height must be finite and > 0"
        );

        let len = source.clone().into_iter().len();

        let build = move |range: Range<usize>| {
            let first = range.start;
            source
                .clone()
                .into_iter()
                .skip(first)
                .take(range.len())
                .enumerate()
                .map(|(i, item)| view_item(first + i, item))
                .collect::<Vec<_>>()
        };

        let rows = Rows {
            len,
            item_height,
            overscan,
            build: Box::new(build),
            children: Vec::new(),
        };

        Self {
            scrollable: Scrollable::with_direction(
                Element::new(rows),
                Direction::Vertical(scrollable::Scrollbar::default()),
            ),
        }
    }

    /// Sets the width of the list.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.scrollable = self.scrollable.width(width);
        self
    }

    /// Sets the height of the list.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.scrollable = self.scrollable.height(height);
        self
    }

    /// Sets the [`widget::Id`] of the list (for scroll operations).
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.scrollable = self.scrollable.id(id);
        self
    }

    /// Sets a closure producing a message whenever the list is scrolled.
    pub fn on_scroll(mut self, on_scroll: impl Fn(Viewport) -> Message + 'a) -> Self {
        self.scrollable = self.scrollable.on_scroll(on_scroll);
        self
    }

    /// Customizes the vertical scrollbar.
    pub fn scrollbar(mut self, scrollbar: scrollable::Scrollbar) -> Self {
        self.scrollable = self.scrollable.direction(Direction::Vertical(scrollbar));
        self
    }
}

// --- LazyList: pure delegation to the inner Scrollable ----------------------

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for LazyList<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: scrollable::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer + 'a,
{
    fn size(&self) -> Size<Length> {
        Widget::<Message, Theme, Renderer>::size(&self.scrollable)
    }

    fn size_hint(&self) -> Size<Length> {
        Widget::<Message, Theme, Renderer>::size_hint(&self.scrollable)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        Widget::<Message, Theme, Renderer>::layout(&mut self.scrollable, tree, renderer, limits)
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
        Widget::<Message, Theme, Renderer>::draw(
            &self.scrollable,
            tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        )
    }

    fn tag(&self) -> tree::Tag {
        Widget::<Message, Theme, Renderer>::tag(&self.scrollable)
    }

    fn state(&self) -> tree::State {
        Widget::<Message, Theme, Renderer>::state(&self.scrollable)
    }

    fn children(&self) -> Vec<Tree> {
        Widget::<Message, Theme, Renderer>::children(&self.scrollable)
    }

    fn diff(&self, tree: &mut Tree) {
        Widget::<Message, Theme, Renderer>::diff(&self.scrollable, tree)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        Widget::<Message, Theme, Renderer>::operate(
            &mut self.scrollable,
            tree,
            layout,
            renderer,
            operation,
        )
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
        Widget::<Message, Theme, Renderer>::update(
            &mut self.scrollable,
            tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        Widget::<Message, Theme, Renderer>::mouse_interaction(
            &self.scrollable,
            tree,
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        Widget::<Message, Theme, Renderer>::overlay(
            &mut self.scrollable,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<LazyList<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: scrollable::Catalog + 'a,
    Renderer: iced::advanced::text::Renderer + 'a,
{
    fn from(list: LazyList<'a, Message, Theme, Renderer>) -> Self {
        Element::new(list)
    }
}

// --- Rows: the windowed content ---------------------------------------------

#[derive(Default)]
struct State {
    initialized: bool,
    /// The range of absolute item indices that currently has children.
    window: Range<usize>,
}

/// Tall as the whole list, but only owns children for the visible window.
struct Rows<'a, Message, Theme, Renderer> {
    len: usize,
    item_height: f32,
    overscan: usize,
    #[expect(clippy::type_complexity)]
    build: Box<dyn Fn(Range<usize>) -> Vec<Element<'a, Message, Theme, Renderer>> + 'a>,
    children: Vec<Element<'a, Message, Theme, Renderer>>,
}

impl<Message, Theme, Renderer> Rows<'_, Message, Theme, Renderer> {
    /// Which items intersect `[offset, offset + view_height)`, with overscan.
    fn window_for(&self, offset: f32, view_height: f32) -> Range<usize> {
        let first = (offset / self.item_height).floor().max(0.0) as usize;
        let last = ((offset + view_height) / self.item_height).ceil().max(0.0) as usize;

        let start = first.saturating_sub(self.overscan).min(self.len);
        let end = last.saturating_add(self.overscan).min(self.len);

        start..end.max(start)
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Rows<'a, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn diff(&self, _tree: &mut Tree) {
        // Diffing is deferred to `layout`, like `Responsive` does: only then
        // do we know which children exist.
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.max().width;
        let width = if width.is_finite() { width } else { 0.0 };

        // Decide the window from what `update` last recorded.
        let window = {
            let state = tree.state.downcast_mut::<State>();

            if !state.initialized {
                state.initialized = true;
                state.window = 0..self.len.min(INITIAL_ROWS);
            }

            // The list may have shrunk/grown since the window was recorded.
            let start = state.window.start.min(self.len);
            let end = state.window.end.min(self.len).max(start);
            state.window = start..end;
            state.window.clone()
        };

        self.children = (self.build)(window.clone());
        tree.diff_children(&self.children);

        let cell = layout::Limits::new(
            Size::new(width, self.item_height),
            Size::new(width, self.item_height),
        );

        let nodes = self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .enumerate()
            .map(|(i, (child, child_tree))| {
                child
                    .as_widget_mut()
                    .layout(child_tree, renderer, &cell)
                    .move_to(Point::new(
                        0.0,
                        (window.start + i) as f32 * self.item_height,
                    ))
            })
            .collect();

        layout::Node::with_children(Size::new(width, self.len as f32 * self.item_height), nodes)
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
        for ((child, child_tree), child_layout) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.as_widget_mut().update(
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

        // `viewport` is the visible part of this widget's own coordinate space.
        let offset = (viewport.y - layout.bounds().y).max(0.0);
        let wanted = self.window_for(offset, viewport.height);

        let state = tree.state.downcast_mut::<State>();

        if state.window != wanted {
            state.window = wanted;
            shell.invalidate_layout();
            shell.request_redraw();
        }
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
        for ((child, child_tree), child_layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            if child_layout.bounds().intersects(viewport) {
                child.as_widget().draw(
                    child_tree,
                    renderer,
                    theme,
                    style,
                    child_layout,
                    cursor,
                    viewport,
                );
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, child_tree), child_layout)| {
                child.as_widget().mouse_interaction(
                    child_tree,
                    child_layout,
                    cursor,
                    viewport,
                    renderer,
                )
            })
            .max()
            .unwrap_or_default()
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((child, child_tree), child_layout) in self
                .children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                child
                    .as_widget_mut()
                    .operate(child_tree, child_layout, renderer, operation);
            }
        });
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::space;

    fn rows(len: usize) -> Element<'static, (), iced::Theme, ()> {
        let build = |range: Range<usize>| {
            range
                .map(|_| Element::from(space().height(Length::Fixed(20.0))))
                .collect::<Vec<_>>()
        };
        Element::new(Rows {
            len,
            item_height: 20.0,
            overscan: 3,
            build: Box::new(build),
            children: Vec::new(),
        })
    }

    #[test]
    fn same_tree_survives_list_shrinking() {
        let limits = layout::Limits::new(Size::ZERO, Size::new(100.0, 400.0));

        let mut big = rows(10_000);
        let mut tree = Tree::new(&big);

        // Pretend `update` scrolled the window far down.
        tree.state.downcast_mut::<State>().window = 9_000..9_030;
        tree.state.downcast_mut::<State>().initialized = true;

        let node = big.as_widget_mut().layout(&mut tree, &(), &limits);
        assert_eq!(node.size().height, 200_000.0);
        assert_eq!(node.children().len(), 30);

        // Same tree, list is now tiny.
        let mut small = rows(5);
        tree.diff(&small);
        let node = small.as_widget_mut().layout(&mut tree, &(), &limits);
        assert_eq!(node.size().height, 100.0);
        assert!(node.children().len() <= 5);
        assert!(tree.state.downcast_ref::<State>().window.end <= 5);

        // And grows again.
        let mut grown = rows(50);
        tree.diff(&grown);
        let node = grown.as_widget_mut().layout(&mut tree, &(), &limits);
        assert_eq!(node.size().height, 1000.0);
    }
}
