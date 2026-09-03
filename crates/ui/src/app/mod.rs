use gpui::{Render, SharedString, Window, div, prelude::*, px};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
};

use crate::{components::titlebar, theme};

pub struct MainView {
    pub text: SharedString,
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (theme_icon, theme_label) = if cx.theme().is_dark() {
            (IconName::Sun, "Switch to light theme")
        } else {
            (IconName::Moon, "Switch to dark theme")
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
            .child(titlebar::render("Magenta"))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .text_color(cx.theme().foreground)
                    .child(
                        div().absolute().top(px(12.)).right(px(12.)).child(
                            Button::new("theme-toggle")
                                .ghost()
                                .small()
                                .icon(theme_icon)
                                .tooltip(theme_label)
                                .on_click(|_, _, cx| {
                                    theme::toggle(cx);
                                }),
                        ),
                    )
                    .child(div().text_xl().child(format!("Hello, {}!", self.text))),
            )
    }
}
