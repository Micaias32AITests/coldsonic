use gpui_kit::{App, AsyncApp, Global};

#[derive(Clone)]
pub struct RT(pub tokio::runtime::Handle);

impl Global for RT {}

impl RT {
    pub fn get(cx: &mut App) -> tokio::runtime::Handle {
        cx.global::<RT>().0.clone()
    }

    pub fn get_async(cx: &mut AsyncApp) -> tokio::runtime::Handle {
        cx.read_global::<RT, _>(|rt, _cx| rt.0.clone())
    }
}
