use gpui::{Render, SharedString, Window, div, prelude::*, px, rgb, rgba};

pub struct Magenta {
    pub text: SharedString,
}

impl Render for Magenta {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .gap_3()
            .bg(rgb(0x0A0B10))
            .size(px(500.))
            .justify_center()
            .items_center()
            .shadow_lg()
            .border_1()
            .border_color(rgba(0xFFFFFF12))
            .text_xl()
            .text_color(rgb(0xF5F5F7))
            .child(format!("Hello, {}!", self.text))
    }
}
