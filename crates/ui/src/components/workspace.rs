use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _, div,
    linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use crate::app::MainView;

pub(crate) fn render(name: SharedString, cx: &mut Context<MainView>) -> AnyElement {
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
                .child(orb(cx))
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
                .child(composer(cx))
                .child(suggestions(cx)),
        )
        .into_any_element()
}

fn ambient_light(cx: &mut Context<MainView>) -> AnyElement {
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

fn orb(cx: &mut Context<MainView>) -> AnyElement {
    div()
        .relative()
        .size(px(120.))
        .rounded_full()
        .overflow_hidden()
        .border_1()
        .border_color(cx.theme().primary.opacity(0.88))
        .bg(linear_gradient(
            155.,
            linear_color_stop(cx.theme().primary.opacity(0.7), 0.),
            linear_color_stop(cx.theme().background, 0.55),
        ))
        .shadow_lg()
        .child(
            div()
                .absolute()
                .top(px(5.))
                .left(px(5.))
                .size(px(108.))
                .rounded_full()
                .bg(linear_gradient(
                    160.,
                    linear_color_stop(cx.theme().muted.opacity(0.92), 0.),
                    linear_color_stop(cx.theme().background, 0.72),
                )),
        )
        .child(
            div()
                .absolute()
                .left(px(-15.))
                .top(px(22.))
                .w(px(105.))
                .h(px(70.))
                .rounded_full()
                .border_2()
                .border_color(cx.theme().primary.opacity(0.86)),
        )
        .child(
            div()
                .absolute()
                .right(px(-17.))
                .top(px(35.))
                .w(px(107.))
                .h(px(62.))
                .rounded_full()
                .border_2()
                .border_color(cx.theme().primary.opacity(0.86)),
        )
        .child(
            div()
                .absolute()
                .top(px(9.))
                .left(px(20.))
                .w(px(54.))
                .h(px(18.))
                .rounded_full()
                .bg(cx.theme().foreground.opacity(0.06)),
        )
        .into_any_element()
}

fn composer(cx: &mut Context<MainView>) -> AnyElement {
    v_flex()
        .w_full()
        .max_w(px(660.))
        .h(px(145.))
        .mt(px(28.))
        .p(px(10.))
        .justify_between()
        .rounded(px(9.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.72))
        .border_t_1()
        .bg(linear_gradient(
            145.,
            linear_color_stop(cx.theme().button.opacity(0.96), 0.),
            linear_color_stop(cx.theme().secondary.opacity(0.82), 1.),
        ))
        .shadow_lg()
        .child(
            v_flex()
                .gap(px(7.))
                .child(
                    h_flex()
                        .gap(px(7.))
                        .child(
                            div()
                                .size(px(34.))
                                .rounded(px(7.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(linear_gradient(
                                    145.,
                                    linear_color_stop(cx.theme().yellow.opacity(0.68), 0.),
                                    linear_color_stop(cx.theme().blue.opacity(0.72), 1.),
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(34.))
                                .rounded(px(7.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().muted)
                                .child(Icon::new(IconName::GalleryVerticalEnd).size_4()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("Describe a new image"),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .gap(px(5.))
                        .child(mini_chip("G", "Nano Banana Pro", cx))
                        .child(mini_chip("", "4:3", cx))
                        .child(mini_chip("◇", "1K", cx))
                        .child(mini_chip("", "Unlimited  ●", cx)),
                )
                .child(
                    Button::new("generate")
                        .primary()
                        .h(px(36.))
                        .px(px(14.))
                        .rounded(px(7.))
                        .icon(IconName::Asterisk)
                        .label("Generate"),
                ),
        )
        .into_any_element()
}

fn mini_chip(prefix: &'static str, label: &'static str, cx: &mut Context<MainView>) -> AnyElement {
    h_flex()
        .h(px(28.))
        .gap(px(5.))
        .px(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.72))
        .bg(cx.theme().muted.opacity(0.82))
        .text_size(px(10.))
        .text_color(cx.theme().muted_foreground)
        .when(!prefix.is_empty(), |this| {
            this.child(
                div()
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .child(prefix),
            )
        })
        .child(label)
        .into_any_element()
}

fn suggestions(cx: &mut Context<MainView>) -> AnyElement {
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

fn suggestion(icon: IconName, label: &'static str, cx: &mut Context<MainView>) -> AnyElement {
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
