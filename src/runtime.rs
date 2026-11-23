use gpui_kit::{App, Global};

#[derive(Clone)]
pub struct RT(pub tokio::runtime::Handle);

impl Global for RT {}

impl RT {
    pub fn get(cx: &mut App) -> tokio::runtime::Handle {
        cx.global::<RT>().0.clone()
    }
}
