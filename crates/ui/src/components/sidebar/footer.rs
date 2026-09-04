use gpui::{
    AnyElement, BoxShadow, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, linear_color_stop, linear_gradient, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use crate::app::MainView;

use super::{Destination, interaction::select_handler, styles};

pub(super) fn render(collapsed: bool, cx: &mut Context<MainView>) -> AnyElement {
    if collapsed {
        compact(cx)
    } else {
        upgrade_card(cx)
    }
}

fn compact(cx: &mut Context<MainView>) -> AnyElement {
    h_flex()
        .w_full()
        .justify_center()
        .child(
            Button::new("upgrade-compact")
                .primary()
                .small()
                .icon(IconName::Asterisk)
                .tooltip("Upgrade to Magenta Pro")
                .on_click(select_handler(Destination::Billing, cx)),
        )
        .into_any_element()
}

fn upgrade_card(cx: &mut Context<MainView>) -> AnyElement {
    let mut card_top = cx.theme().button_hover;
    card_top.s *= 0.38;
    let mut card_bottom = cx.theme().button;
    card_bottom.s *= 0.24;
    card_bottom.l = (card_bottom.l + 0.025).min(1.);

    v_flex()
        .id("upgrade-card")
        .w_full()
        .h(px(128.))
        .justify_between()
        .p(px(10.))
        .rounded(px(12.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(card_top.opacity(0.98), 0.),
            linear_color_stop(card_bottom.opacity(0.99), 1.),
        ))
        .border_1()
        .border_color(cx.theme().primary.opacity(0.38))
        .shadow(styles::illuminated_card_shadow(cx))
        .child(styles::card_bevel_light(cx))
        .child(card_header(cx))
        .child(card_copy(cx))
        .child(card_actions(cx))
        .into_any_element()
}

fn card_header(cx: &mut Context<MainView>) -> AnyElement {
    h_flex()
        .relative()
        .items_center()
        .justify_between()
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .rounded_full()
                        .bg(cx.theme().tokens.primary.background)
                        .text_color(cx.theme().primary_foreground)
                        .child(Icon::new(IconName::Star).xsmall()),
                )
                .child(
                    div()
                        .font_medium()
                        .text_size(px(13.))
                        .text_color(cx.theme().foreground)
                        .child("Upgrade Pro!"),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded_full()
                .border_1()
                .border_color(cx.theme().border)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::WindowClose).xsmall()),
        )
        .into_any_element()
}

fn card_copy(cx: &mut Context<MainView>) -> AnyElement {
    div()
        .relative()
        .max_w(px(170.))
        .text_size(px(10.))
        .line_height(px(14.))
        .text_color(cx.theme().muted_foreground)
        .child("Upgrade to Pro and elevate your experience today")
        .into_any_element()
}

fn card_actions(cx: &mut Context<MainView>) -> AnyElement {
    h_flex()
        .relative()
        .items_center()
        .gap(px(8.))
        .child(
            Button::new("upgrade")
                .primary()
                .small()
                .h(px(28.))
                .px(px(14.))
                .rounded(px(7.))
                .bg(linear_gradient(
                    100.,
                    linear_color_stop(cx.theme().primary, 0.),
                    linear_color_stop(cx.theme().blue, 1.),
                ))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(3.), cx.theme().primary.opacity(0.22))
                        .blur_radius(px(9.)),
                    BoxShadow::new(px(0.), px(1.), cx.theme().foreground.opacity(0.2)).inset(),
                ])
                .icon(IconName::Star)
                .label("Upgrade")
                .on_click(select_handler(Destination::Billing, cx)),
        )
        .child(
            Button::new("learn-more")
                .ghost()
                .h(px(28.))
                .px(px(8.))
                .label("Learn More")
                .on_click(select_handler(Destination::Billing, cx)),
        )
        .into_any_element()
}
