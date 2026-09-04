use gpui::{
    AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};

use crate::app::MainView;

pub(super) fn render(collapsed: bool, cx: &mut Context<MainView>) -> AnyElement {
    let toggle_label = if collapsed {
        "Expand sidebar"
    } else {
        "Collapse sidebar"
    };
    let toggle_icon = if collapsed {
        IconName::PanelLeftOpen
    } else {
        IconName::PanelLeftClose
    };

    h_flex()
        .w_full()
        .h(px(57.))
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .when(!collapsed, |this| {
            this.child(
                div()
                    .text_size(px(20.))
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .child("magenta"),
            )
        })
        .when(collapsed, |this| this.justify_center())
        .child(
            Button::new("sidebar-toggle")
                .ghost()
                .small()
                .icon(toggle_icon)
                .tooltip(toggle_label)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.toggle_sidebar(cx);
                })),
        )
        .into_any_element()
}
