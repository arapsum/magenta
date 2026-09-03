use gpui::{Render, SharedString, Window, div, prelude::*, rgb};

use crate::components::titlebar;

pub struct MainView {
    pub text: SharedString,
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0A0B10))
            .child(titlebar::render("Magenta"))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .text_xl()
                    .text_color(rgb(0xF5F5F7))
                    .child(format!("Hello, {}!", self.text)),
            )
    }
}
