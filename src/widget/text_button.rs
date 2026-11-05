//! A button that reads as plain, clickable text.

use iced::widget::button;

use crate::prelude::*;

/// A transparent, zero-padding button styled like inline text.
///
/// Use this for navigation or playback actions where a filled button would be
/// too visually heavy, such as artist, album, and track names. The caller owns
/// the label's size, wrapping, and width; the button only contributes the click
/// target and hover feedback.
pub fn text_button<'a, Message>(
    content: impl Into<Element<'a, Message>>,
) -> button::Button<'a, Message> {
    button(content)
        .style(button::text)
        .padding(Padding::from([2, 0]))
}
