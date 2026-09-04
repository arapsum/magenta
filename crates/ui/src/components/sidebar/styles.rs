use gpui::{
    AnyElement, App, BoxShadow, IntoElement as _, ParentElement as _, Styled as _, div,
    linear_color_stop, linear_gradient, px,
};
use gpui_component::ActiveTheme as _;

pub(super) fn illuminated_button_shadow(cx: &App) -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.), px(0.), cx.theme().primary.opacity(0.12)).blur_radius(px(12.)),
        BoxShadow::new(px(0.), px(4.), cx.theme().background.opacity(0.66)).blur_radius(px(9.)),
        BoxShadow::new(px(0.), px(1.), cx.theme().foreground.opacity(0.1)).inset(),
        BoxShadow::new(px(0.), px(-1.), cx.theme().background.opacity(0.72)).inset(),
    ]
}

pub(super) fn compact_bevel_light(cx: &App) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .overflow_hidden()
        .rounded(px(7.))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .right(px(14.))
                .w(px(120.))
                .h(px(1.))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(7.), cx.theme().primary.opacity(0.34))
                        .blur_radius(px(17.))
                        .spread_radius(px(2.)),
                ]),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(12.))
                .w(px(133.))
                .h(px(1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(cx.theme().primary.opacity(0.04), 0.),
                    linear_color_stop(cx.theme().primary.opacity(0.66), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(145.))
                .right(px(12.))
                .h(px(1.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(cx.theme().primary.opacity(0.66), 0.),
                    linear_color_stop(cx.theme().primary.opacity(0.04), 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .top(px(1.))
                .left(px(1.))
                .right(px(1.))
                .h(px(10.))
                .rounded(px(6.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(cx.theme().foreground.opacity(0.055), 0.),
                    linear_color_stop(cx.theme().foreground.opacity(0.), 1.),
                )),
        )
        .into_any_element()
}
