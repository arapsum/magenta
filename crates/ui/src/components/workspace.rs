use gpui::{
    AnyElement, Context, Entity, IntoElement, ParentElement as _, SharedString, Styled as _, div,
    linear_color_stop, linear_gradient, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex, v_flex,
};

use crate::app::MainView;
use crate::components::{orb, prompt_input::PromptComposer};

pub fn render(
    name: &SharedString,
    composer: Entity<PromptComposer>,
    cx: &Context<'_, MainView>,
) -> AnyElement {
    div()
        .relative()
        .flex()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .bg(cx.theme().tokens.background.background)
        .text_color(cx.theme().foreground)
        .child(ambient_light(cx))
        .child(
            v_flex()
                .relative()
                .w_full()
                .max_w(px(760.))
                .items_center()
                .gap_0()
                .pb(px(22.))
                .child(orb::render())
                .child(
                    v_flex()
                        .items_center()
                        .gap(px(5.))
                        .mt(px(24.))
                        .child(
                            div()
                                .text_size(px(24.))
                                .line_height(px(30.))
                                .font_semibold()
                                .child(format!("Hi {name}")),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .line_height(px(20.))
                                .text_color(cx.theme().muted_foreground)
                                .child("Ask anything about your meetings?"),
                        ),
                )
                .child(div().w_full().max_w(px(660.)).mt(px(28.)).child(composer))
                .child(suggestions(cx)),
        )
        .into_any_element()
}

fn ambient_light(cx: &Context<'_, MainView>) -> AnyElement {
    div()
        .absolute()
        .top(px(-160.))
        .right(px(-80.))
        .size(px(520.))
        .rounded_full()
        .bg(linear_gradient(
            145.,
            linear_color_stop(cx.theme().primary.opacity(0.12), 0.),
            linear_color_stop(cx.theme().background.opacity(0.), 1.),
        ))
        .opacity(0.75)
        .into_any_element()
}

fn suggestions(cx: &Context<'_, MainView>) -> AnyElement {
    h_flex()
        .mt(px(12.))
        .gap(px(7.))
        .child(suggestion(IconName::Asterisk, "AI for Real Work", cx))
        .child(suggestion(IconName::BookOpen, "Smart Summaries", cx))
        .child(suggestion(
            IconName::TriangleAlert,
            "Ask Anything. Get Results.",
            cx,
        ))
        .child(suggestion(
            IconName::Network,
            "Fast. Private. Reliable.",
            cx,
        ))
        .into_any_element()
}

fn suggestion(icon: IconName, label: &'static str, cx: &Context<'_, MainView>) -> AnyElement {
    h_flex()
        .h(px(30.))
        .gap(px(6.))
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.62))
        .bg(cx.theme().secondary.opacity(0.74))
        .text_size(px(10.))
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon).xsmall())
        .child(label)
        .into_any_element()
}
