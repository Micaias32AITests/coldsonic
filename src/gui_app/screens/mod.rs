use gpui_kit::{AnyElement, div, prelude::*};

pub mod album_detail;
pub mod albums;
pub mod artist_detail;
pub mod artists;
pub mod tracks;

fn nothing_to_see_here() -> AnyElement {
    div()
        .h_full()
        .w_full()
        .child("Nothing to see here...")
        .into_any_element()
}
